//! HNS ⇄ BTC exchange commands (docs/specs/2026-11-07-hns-btc-exchange.md
//! §9). IO shells: they read the database and the chains, ask the secure
//! window, and call the pure layer (`noncustodial::exchange`) and its IO
//! helpers (`exchange_io`).
//!
//! Money safety in the order of a Commit: everything the party signs is
//! stored **before** its lock's funding is sent, so a crash between the two
//! never leaves coins in a lock without the presigned way back.

use serde::Serialize;
use tauri::State;

use crate::commands::draft_ctx::{self, Ctx};
use crate::error::AppError;
use crate::exchange_io::btc_source::BtcSource;
use crate::exchange_io::store::{self, Trade};
use crate::exchange_io::{btc_observe, btc_wallet, env, runner};
use crate::noncustodial::exchange::engine::{self, Role, Status};
use crate::noncustodial::exchange::relay_msg::{
    self, Body, LockAnnouncement, Message, Offer, Side, SignedOffer, TakeRequest,
};
use crate::noncustodial::exchange::{btc_account, btc_htlc, commit, hns_htlc, keys, price, review};
use crate::noncustodial::network::Network;

use crate::noncustodial::session::{session_ttl_ms, SignerSession};
use crate::AppState;

/// R17: the rule and its sentences live in `exchange_io::env`, so the sync
/// step applies the same one; re-exported for the tests and the overview.
pub use crate::exchange_io::env::{
    refusal, NEEDS_LOCAL_NODE, NEEDS_NODE, NEEDS_RECOVERY_PHRASE, NEEDS_SENDING_NODE,
};

/// The paired Bitcoin network as the contract spells it.
pub fn btc_network_name(n: Network) -> &'static str {
    match btc_htlc::btc_network(n) {
        bitcoin::Network::Bitcoin => "bitcoin",
        bitcoin::Network::Testnet => "testnet",
        _ => "regtest",
    }
}

fn lock<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, rusqlite::Connection>, AppError> {
    state.db.lock().map_err(|e| AppError::Lock(e.to_string()))
}

fn db_path(state: &State<'_, AppState>) -> Result<String, AppError> {
    // An in-memory database (tests) has no path; the step then opens nothing.
    Ok(lock(state)?.path().unwrap_or("").to_owned())
}

fn now() -> u64 {
    env::now_secs()
}

fn with_signer<T>(
    state: &State<'_, AppState>,
    ctx: &Ctx,
    f: impl FnOnce(&mut SignerSession) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut slot = state
        .signer
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?;
    let session = slot.as_mut().ok_or(AppError::WalletLocked)?;
    session.authorize(&ctx.profile_id, session_ttl_ms(&ctx.settings))?;
    f(session)
}

/// The context of a command that may trade: the gates of R17 passed.
fn trading_ctx(state: &State<'_, AppState>) -> Result<Ctx, AppError> {
    let ctx = draft_ctx::load_ctx(state)?;
    if let Some(r) = env::node_refusal(&ctx.profile_kind, &ctx.node, &ctx.settings) {
        return Err(AppError::InvalidInput(r.into()));
    }
    Ok(ctx)
}

fn account_of(conn: &rusqlite::Connection, ctx: &Ctx) -> Result<store::Account, AppError> {
    store::account(conn, &ctx.profile_id)?
        .ok_or_else(|| AppError::InvalidInput("enable the exchange first".into()))
}

// ---------------------------------------------------------------- overview

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeOverview {
    pub network: String,
    pub available: bool,
    pub refusal: Option<String>,
    pub enabled: bool,
    pub relay_url: String,
    pub watchtower_url: Option<String>,
    pub btc_sources: Vec<String>,
    pub btc_network: String,
    pub bitcoind_auth_set: bool,
    /// The Nostr relays as set (empty: the defaults below).
    pub nostr_relays: Vec<String>,
    /// The network's default Nostr relays.
    pub default_nostr_relays: Vec<String>,
    /// Where offers and messages go: `"relay"` (the custom relay),
    /// `"nostr"`, or `"none"` (neither is set).
    pub meeting_point: String,
}

/// What the page needs to know about a profile before any key is involved:
/// who it is, its network, the settings and its node. Unlike
/// [`draft_ctx::load_ctx`] it also loads for a watch-only or Ledger profile,
/// so the page can say why such a profile cannot trade (R17) and still show
/// the order book, instead of failing with "active profile is watch-only"
/// (QA 2026-11-11, Q9).
pub(crate) struct View {
    pub profile_id: String,
    pub profile_kind: String,
    pub network: Network,
    pub settings: std::collections::HashMap<String, String>,
    pub node: crate::noncustodial::rpc::NodeRpcClient,
}

impl View {
    pub(crate) fn of(ctx: &Ctx) -> Self {
        View {
            profile_id: ctx.profile_id.clone(),
            profile_kind: ctx.profile_kind.clone(),
            network: ctx.network,
            settings: ctx.settings.clone(),
            node: ctx.node.clone(),
        }
    }

    /// The active profile's view, whatever its kind.
    pub(crate) fn load(conn: &rusqlite::Connection) -> Result<Self, AppError> {
        let profile = draft_ctx::active_profile(conn)?;
        Ok(View {
            network: crate::noncustodial::derivation::network_from_profile(&profile.network)?,
            settings: crate::db::queries::get_settings(conn)?,
            node: crate::noncustodial::rpc::NodeRpcClient::for_profile(conn, &profile.id)?,
            profile_id: profile.id,
            profile_kind: profile.kind,
        })
    }
}

pub(crate) fn overview_from(
    conn: &rusqlite::Connection,
    ctx: &Ctx,
) -> Result<ExchangeOverview, AppError> {
    overview_of(conn, &View::of(ctx))
}

pub(crate) fn overview_of(
    conn: &rusqlite::Connection,
    v: &View,
) -> Result<ExchangeOverview, AppError> {
    let cfg = env::config(&v.settings, v.network);
    let r = env::node_refusal(&v.profile_kind, &v.node, &v.settings);
    Ok(ExchangeOverview {
        network: v.network.as_str().to_owned(),
        available: r.is_none(),
        refusal: r.map(str::to_owned),
        enabled: store::account(conn, &v.profile_id)?.is_some(),
        meeting_point: env::meeting_point(&cfg, v.network).as_str().to_owned(),
        default_nostr_relays: env::default_nostr_relays(v.network),
        nostr_relays: cfg.nostr_relays.clone(),
        relay_url: cfg.relay_url,
        watchtower_url: Some(cfg.watchtower_url).filter(|w| !w.is_empty()),
        btc_sources: cfg.btc_sources,
        btc_network: btc_network_name(v.network).to_owned(),
        bitcoind_auth_set: cfg.bitcoind_auth.is_some(),
    })
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_overview(state: State<'_, AppState>) -> Result<ExchangeOverview, AppError> {
    let conn = lock(&state)?;
    overview_of(&conn, &View::load(&conn)?)
}

/// Store the BTC account's and the swap branch's public keys (needs the
/// unlocked session; no key is stored, nothing is signed).
#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_enable(state: State<'_, AppState>) -> Result<ExchangeOverview, AppError> {
    let ctx = trading_ctx(&state)?;
    let (btc_xpub, swap_xpub) = with_signer(&state, &ctx, |s| {
        let m = s.master()?;
        Ok((
            btc_account::account_xpub(m, ctx.network, btc_account::ACCOUNT)?,
            keys::branch_xpub(m, ctx.network, ctx.account)?,
        ))
    })?;
    let conn = lock(&state)?;
    store::insert_account(
        &conn,
        &store::Account {
            profile_id: ctx.profile_id.clone(),
            network: ctx.network.as_str().to_owned(),
            btc_xpub,
            swap_xpub,
            account: ctx.account,
            next_trade_index: 0,
            next_btc_receive: 0,
            next_btc_change: 0,
        },
        now(),
    )?;
    overview_from(&conn, &ctx)
}

// ---------------------------------------------------------------- settings

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_update_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    relay_url: String,
    watchtower_url: String,
    btc_sources: Vec<String>,
    bitcoind_auth: Option<String>,
    nostr_relays: Vec<String>,
) -> Result<ExchangeOverview, AppError> {
    let ctx = draft_ctx::load_ctx(&state)?;
    let new = env::with_nostr(
        env::validate(
            ctx.network,
            &relay_url,
            &watchtower_url,
            &btc_sources,
            bitcoind_auth.as_deref(),
        )?,
        &nostr_relays,
    )?;
    let old = env::config(&ctx.settings, ctx.network);
    let rows = settings_rows(&old, &new);
    if rows.is_empty() {
        let conn = lock(&state)?;
        return overview_from(&conn, &ctx);
    }
    crate::commands::secure_confirm::confirm_rows(
        &app,
        "Change exchange settings",
        "Bitcoin sources are not trusted: Namehold checks every lock by proof of work and needs every source to agree. The Nostr relays, the relay and the watchtower hold no key and no secret.",
        serde_json::json!({ "rows": rows }),
    )
    .await?;
    let n = ctx.network;
    {
        // The DB mutex is released before `load_ctx`, which takes it again:
        // holding it across that call deadlocked the whole app (QA Q1,
        // 2026-11-09: every later command hung after a confirmed save).
        let conn = lock(&state)?;
        // Written and logged like every other setting; the login is logged as ***.
        let put = |k: &str, v: &str| -> Result<(), AppError> {
            let key = env::key(k, n);
            crate::db::queries::set_setting(&conn, &key, v)?;
            crate::commands::settings::log_setting_change(&conn, &key, v)
        };
        put(env::RELAY_KEY, &new.relay_url)?;
        put(env::WATCHTOWER_KEY, &new.watchtower_url)?;
        put(env::SOURCES_KEY, &new.btc_sources.join("\n"))?;
        put(env::NOSTR_KEY, &new.nostr_relays.join("\n"))?;
        if let Some(a) = &new.bitcoind_auth {
            put(env::BITCOIND_AUTH_KEY, a)?;
        }
    }
    let ctx = draft_ctx::load_ctx(&state)?;
    let conn = lock(&state)?;
    overview_from(&conn, &ctx)
}

/// One row per changed setting; the bitcoind login only as set/changed/cleared.
pub fn settings_rows(old: &env::Config, new: &env::Config) -> Vec<serde_json::Value> {
    let row = |l: &str, v: String| serde_json::json!({ "label": l, "value": v });
    let shown = |s: &str| {
        if s.is_empty() {
            "none".to_owned()
        } else {
            s.to_owned()
        }
    };
    let mut rows = Vec::new();
    if old.relay_url != new.relay_url {
        rows.push(row(
            "Relay",
            format!("{} → {}", shown(&old.relay_url), shown(&new.relay_url)),
        ));
    }
    if old.watchtower_url != new.watchtower_url {
        rows.push(row(
            "Watchtower",
            format!(
                "{} → {}",
                shown(&old.watchtower_url),
                shown(&new.watchtower_url)
            ),
        ));
    }
    if old.nostr_relays != new.nostr_relays {
        let list = |l: &[String]| {
            if l.is_empty() {
                "the defaults".to_owned()
            } else {
                l.join(", ")
            }
        };
        rows.push(row(
            "Nostr relays",
            format!("{} → {}", list(&old.nostr_relays), list(&new.nostr_relays)),
        ));
    }
    if old.btc_sources != new.btc_sources {
        rows.push(row(
            "Bitcoin sources",
            format!(
                "{} → {}",
                shown(&old.btc_sources.join(", ")),
                shown(&new.btc_sources.join(", "))
            ),
        ));
    }
    match (&old.bitcoind_auth, &new.bitcoind_auth) {
        (_, None) => {}
        (None, Some(a)) if !a.is_empty() => rows.push(row("bitcoind login", "set".into())),
        (Some(_), Some(a)) if a.is_empty() => rows.push(row("bitcoind login", "cleared".into())),
        (Some(o), Some(a)) if o != a => rows.push(row("bitcoind login", "changed".into())),
        _ => {}
    }
    rows
}

// ------------------------------------------------------------- BTC account

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BtcBalance {
    pub confirmed_sats: u64,
    pub unconfirmed_sats: u64,
    pub receive_address: String,
    pub locked_in_trades_sats: u64,
    pub checked_at: u64,
    pub error: Option<String>,
}

fn built_sources(ctx: &Ctx) -> Result<Vec<Box<dyn BtcSource>>, AppError> {
    let cfg = env::config(&ctx.settings, ctx.network);
    let s = env::sources(&cfg)?;
    if s.is_empty() {
        return Err(AppError::InvalidInput(
            "no Bitcoin source is set: add one in Exchange settings".into(),
        ));
    }
    Ok(s)
}

/// BTC committed to this profile's open BTC locks.
pub fn locked_in_trades(trades: &[Trade]) -> u64 {
    trades
        .iter()
        .filter(|t| t.role == Role::Buyer && !t.finished)
        .filter(|t| t.holdings.as_ref().is_some_and(|h| h.committed))
        .filter(|t| !t.status.as_ref().is_some_and(Status::is_terminal))
        .map(|t| t.btc_sats)
        .sum()
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_btc_balance(state: State<'_, AppState>) -> Result<BtcBalance, AppError> {
    let ctx = draft_ctx::load_ctx(&state)?;
    let (acct, trades) = {
        let conn = lock(&state)?;
        (
            account_of(&conn, &ctx)?,
            store::trades(&conn, &ctx.profile_id)?,
        )
    };
    let receive =
        btc_account::address_at(&acct.btc_xpub, ctx.network, 0, acct.next_btc_receive)?.to_string();
    let mut b = BtcBalance {
        confirmed_sats: 0,
        unconfirmed_sats: 0,
        receive_address: receive,
        locked_in_trades_sats: locked_in_trades(&trades),
        checked_at: now(),
        error: None,
    };
    let srcs = match built_sources(&ctx) {
        Ok(s) => s,
        Err(e) => {
            b.error = Some(e.to_string());
            return Ok(b);
        }
    };
    match btc_wallet::coins(srcs[0].as_ref(), &acct, ctx.network).await {
        Ok(coins) => {
            for c in coins {
                if c.height.is_some() {
                    b.confirmed_sats += c.value;
                } else {
                    b.unconfirmed_sats += c.value;
                }
            }
        }
        Err(e) => b.error = Some(e.to_string()),
    }
    Ok(b)
}

/// The median of the sources' fastest rates. The presigned ladder is built
/// from it and cannot be re-signed (spec §6), so no estimate at all refuses
/// the Commit rather than signing at a guessed rate.
/// The Bitcoin fee rate of a send, and whether it is an estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BtcRate {
    pub sat_per_vb: u64,
    /// No source had an estimate: the rate is the nodes' relay minimum times
    /// [`FLOOR_RATE_FACTOR`], and the secure window says so.
    pub fallback: bool,
}

/// How far above the relay minimum a fallback rate is set.
pub const FLOOR_RATE_FACTOR: u64 = 2;

/// The median of the sources' estimates, or, when none has one, a rate from
/// their relay minimum (spec R27; QA 2026-11-09 Q3: a Bitcoin Core without
/// fee history could send nothing and gave the user nothing to do).
pub fn pick_btc_rate(
    estimates: &[u64],
    floors: &[u64],
    errors: &[String],
) -> Result<BtcRate, AppError> {
    let mut rates = estimates.to_vec();
    rates.sort();
    // Capped: one lying source cannot make the ladder absurd, and the cap is
    // shown in the fees.
    if let Some(median) = rates.get(rates.len() / 2).copied() {
        return Ok(BtcRate {
            sat_per_vb: median.clamp(1, 500),
            fallback: false,
        });
    }
    // The highest floor: a transaction every node of ours will relay.
    match floors.iter().copied().max() {
        Some(floor) => Ok(BtcRate {
            sat_per_vb: floor.saturating_mul(FLOOR_RATE_FACTOR).clamp(1, 500),
            fallback: true,
        }),
        None => Err(AppError::InvalidInput(format!(
            "no Bitcoin source gave a fee estimate or a minimum fee, so nothing was signed: {}",
            errors.join("; ")
        ))),
    }
}

async fn btc_rate(srcs: &[Box<dyn BtcSource>]) -> Result<BtcRate, AppError> {
    let mut estimates = Vec::new();
    let mut errors = Vec::new();
    for s in srcs {
        match s.fee_rate().await {
            Ok(r) => estimates.push(r),
            Err(e) => errors.push(e.to_string()),
        }
    }
    let mut floors = Vec::new();
    if estimates.is_empty() {
        for s in srcs {
            match s.floor_rate().await {
                Ok(r) => floors.push(r),
                Err(e) => errors.push(e.to_string()),
            }
        }
    }
    pick_btc_rate(&estimates, &floors, &errors)
}

/// The secure window's rows with the fee rate's row added when the rate is
/// not an estimate, so the user confirms knowing it may confirm slowly.
fn with_rate_note(mut rows: serde_json::Value, rate: BtcRate) -> serde_json::Value {
    if rate.fallback {
        if let Some(list) = rows.get_mut("rows").and_then(|r| r.as_array_mut()) {
            list.push(serde_json::json!({
                "label": "Bitcoin fee rate",
                "value": btc_rate_note(rate),
            }));
        }
    }
    rows
}

pub fn btc_rate_note(rate: BtcRate) -> String {
    format!(
        "{} sat/vB: your Bitcoin node has no fee estimate yet, so this is {} times its minimum. \
         It may confirm slowly while blocks are full",
        rate.sat_per_vb, FLOOR_RATE_FACTOR
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    pub txid: String,
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_btc_withdraw(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    to_address: String,
    amount_sats: Option<u64>,
) -> Result<Sent, AppError> {
    let ctx = trading_ctx(&state)?;
    // Unlocked before the secure window opens, not after it.
    with_signer(&state, &ctx, |_| Ok(()))?;
    let to = btc_htlc::parse_address(ctx.network, &to_address)?;
    let acct = {
        let conn = lock(&state)?;
        account_of(&conn, &ctx)?
    };
    let srcs = built_sources(&ctx)?;
    let coins = btc_wallet::coins(srcs[0].as_ref(), &acct, ctx.network).await?;
    let picked = btc_rate(&srcs).await?;
    let rate = picked.sat_per_vb;
    let confirmed: Vec<_> = coins.into_iter().filter(|c| c.height.is_some()).collect();
    let f = match amount_sats {
        Some(a) => {
            let change = {
                let conn = lock(&state)?;
                let i = store::take_btc_index(&conn, &ctx.profile_id, 1)?;
                btc_account::address_at(&acct.btc_xpub, ctx.network, 1, i)?
            };
            btc_account::fund(
                &confirmed,
                vec![bitcoin::TxOut {
                    value: bitcoin::Amount::from_sat(a),
                    script_pubkey: to.script_pubkey(),
                }],
                &change,
                rate,
            )?
        }
        // Everything: every confirmed coin into one output, with no change.
        None => btc_account::fund_all(&confirmed, &to, rate)?,
    };
    let sending = f.tx.output[0].value.to_sat();
    crate::commands::secure_confirm::confirm_rows(
        &app,
        "Send BTC",
        "From this wallet's BTC account.",
        with_rate_note(
            serde_json::json!({ "rows": [
                { "label": "To", "value": to.to_string() },
                { "label": "Amount", "value": review::sats_text(sending) },
                { "label": "Network fee", "value": review::sats_text(f.fee) },
                { "label": "Change back to this wallet", "value": review::sats_text(f.change) },
            ]}),
            picked,
        ),
    )
    .await?;
    let tx = with_signer(&state, &ctx, |s| {
        btc_account::sign_funding(&f, s.master()?, ctx.network, btc_account::ACCOUNT)
    })?;
    let hex_tx = btc_htlc::encode_tx(&tx);
    runner::send_btc(&srcs, &hex_tx).await?;
    Ok(Sent {
        txid: tx.compute_txid().to_string(),
    })
}

// ------------------------------------------------------------------ offers

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferRow {
    pub id: String,
    pub side: Side,
    pub price: String,
    pub min_hns: String,
    pub max_hns: String,
    pub min_sats: u64,
    pub max_sats: u64,
    pub own: bool,
    pub maker_key: String,
    pub expires_at: u64,
    pub live: bool,
}

/// Whole HNS with six decimals and no unit, as the bridge carries amounts
/// (`src/lib/hnsAmount.ts` parses exactly this).
pub fn hns_text(doos: u64) -> String {
    format!("{}.{:06}", doos / 1_000_000, doos % 1_000_000)
}

pub fn offer_row(o: &SignedOffer, own: bool, live: bool) -> OfferRow {
    let f = &o.offer;
    OfferRow {
        id: f.id(),
        side: f.side,
        price: price::format_sats_per_hns(f.price_msat_per_hns),
        min_hns: hns_text(f.min_hns),
        max_hns: hns_text(f.max_hns),
        // An offer whose limit times its price overflows is one no trade can
        // reach: shown as the extremes, and refused by `terms::check` at a take.
        min_sats: price::btc_for(f.min_hns, f.price_msat_per_hns).unwrap_or(0),
        max_sats: price::btc_for(f.max_hns, f.price_msat_per_hns).unwrap_or(u64::MAX),
        own,
        maker_key: f.maker_relay_pubkey.chars().take(8).collect(),
        expires_at: f.expires_at,
        live,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketOffers {
    pub offers: Vec<OfferRow>,
    pub fetched_at: u64,
    pub error: Option<String>,
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_list_offers(state: State<'_, AppState>) -> Result<MarketOffers, AppError> {
    // Browsing needs no key: a watch-only profile reads the order book too.
    let (ctx, own): (View, Vec<String>) = {
        let conn = lock(&state)?;
        let v = View::load(&conn)?;
        let own = store::offers(&conn, &v.profile_id)?
            .into_iter()
            .map(|o| o.id)
            .collect();
        (v, own)
    };
    let cfg = env::config(&ctx.settings, ctx.network);
    let t = now();
    let Some(relay) = env::relay(&cfg, ctx.network)? else {
        return Ok(MarketOffers {
            offers: Vec::new(),
            fetched_at: t,
            error: Some(env::NO_MEETING_POINT.into()),
        });
    };
    match relay.offers(ctx.network.as_str(), t).await {
        Ok((list, _dropped)) => Ok(MarketOffers {
            offers: list
                .iter()
                .map(|o| {
                    let mine = own.contains(&o.offer.id());
                    offer_row(o, mine, true)
                })
                .collect(),
            fetched_at: t,
            error: None,
        }),
        Err(e) => Ok(MarketOffers {
            offers: Vec::new(),
            fetched_at: t,
            error: Some(e.to_string()),
        }),
    }
}

/// How long a new offer lives before the maker renews it by making another.
pub const OFFER_LIFETIME_SECS: u64 = 7 * 24 * 3600;
/// Hashes a Buyer-maker pools for an offer: that many takes can be answered
/// without the phrase.
pub const HASH_POOL: u32 = 20;

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_create_offer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    side: Side,
    price: String,
    min_hns: String,
    max_hns: String,
) -> Result<OfferRow, AppError> {
    let ctx = trading_ctx(&state)?;
    // Unlocked before the secure window opens, not after it.
    if side == Side::Buy {
        with_signer(&state, &ctx, |_| Ok(()))?;
    }
    let msat = price::parse_sats_per_hns(&price)?;
    let min = parse_hns(&min_hns)?;
    let max = parse_hns(&max_hns)?;
    if max < min {
        return Err(AppError::InvalidInput(
            "the largest trade is below the smallest".into(),
        ));
    }
    let p = env::policy(ctx.network);
    let (lo, hi) = (price::btc_for(min, msat)?, price::btc_for(max, msat)?);
    if lo < p.min_btc_sats || hi > p.max_btc_sats {
        return Err(AppError::InvalidInput(format!(
            "each trade must be worth {} to {} sats",
            p.min_btc_sats, p.max_btc_sats
        )));
    }
    let cfg = env::config(&ctx.settings, ctx.network);
    let relay = env::relay(&cfg, ctx.network)?
        .ok_or_else(|| AppError::InvalidInput(env::NO_MEETING_POINT.into()))?;
    {
        let conn = lock(&state)?;
        account_of(&conn, &ctx)?;
    }
    let what = match side {
        Side::Sell => "Sell HNS for BTC",
        Side::Buy => "Buy HNS with BTC",
    };
    crate::commands::secure_confirm::confirm_rows(
        &app,
        "Publish an offer",
        "Nothing is signed or locked now. Each trade someone starts on this offer asks you to confirm it here, and needs Namehold open.",
        serde_json::json!({ "rows": [
            { "label": "Offer", "value": what },
            { "label": "Price", "value": format!("{} sats per HNS", price::format_sats_per_hns(msat)) },
            { "label": "Each trade", "value": format!("{} to {} ({} to {} sats)",
                crate::noncustodial::types::doos_to_hns_string(min), crate::noncustodial::types::doos_to_hns_string(max), lo, hi) },
            { "label": "Relay", "value": cfg.relay_url.clone() },
        ]}),
    )
    .await?;
    if side == Side::Buy {
        // Hashes for the next takes, from the phrase now, so the daemon can
        // answer them without it.
        let first = {
            let conn = lock(&state)?;
            store::take_trade_indexes(&conn, &ctx.profile_id, HASH_POOL)?
        };
        let hashes = with_signer(&state, &ctx, |s| {
            let m = s.master()?;
            (first..first + HASH_POOL)
                .map(|i| {
                    let n = keys::new_secret_nonce();
                    Ok((
                        i,
                        keys::trade_keys(m, ctx.network, ctx.account, i)?.hash(&n),
                        n,
                    ))
                })
                .collect::<Result<Vec<_>, AppError>>()
        })?;
        let conn = lock(&state)?;
        store::insert_hashes(&conn, &ctx.profile_id, &hashes)?;
    }
    let key = relay_msg::new_relay_key();
    let t = now();
    let offer = Offer {
        version: relay_msg::MSG_VERSION,
        network: ctx.network.as_str().to_owned(),
        side,
        price_msat_per_hns: msat,
        min_hns: min,
        max_hns: max,
        maker_relay_pubkey: relay_msg::relay_pubkey_hex(&key),
        created_at: t,
        expires_at: t + OFFER_LIFETIME_SECS,
        nonce: relay_msg::random_hex16(),
    };
    let signed = offer.sign(&key)?;
    let own = store::OwnOffer {
        id: signed.offer.id(),
        network: ctx.network.as_str().to_owned(),
        side,
        signed: signed.clone(),
        relay_key: key,
        live: true,
        last_published_at: None,
        last_error: None,
    };
    {
        let conn = lock(&state)?;
        store::insert_offer(&conn, &ctx.profile_id, &own, t)?;
    }
    let err = relay
        .publish_offer(&signed, &own.relay_key)
        .await
        .err()
        .map(|e| e.to_string());
    {
        let conn = lock(&state)?;
        store::set_offer_published(&conn, &own.id, t, err.as_deref())?;
    }
    if let Some(e) = err {
        return Err(AppError::Other(format!(
            "the offer is saved but the relay refused it: {e}"
        )));
    }
    Ok(offer_row(&signed, true, true))
}

fn parse_hns(text: &str) -> Result<u64, AppError> {
    let bad = || AppError::InvalidInput(format!("'{text}' is not an amount of HNS"));
    let v = price::parse_fixed(text, 6, bad)?;
    if v == 0 || v > 2_040_000_000 * 1_000_000 {
        return Err(bad());
    }
    Ok(v)
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_withdraw_offer(
    state: State<'_, AppState>,
    offer_id: String,
) -> Result<(), AppError> {
    let ctx = draft_ctx::load_ctx(&state)?;
    let own = {
        let conn = lock(&state)?;
        store::offers(&conn, &ctx.profile_id)?
            .into_iter()
            .find(|o| o.id == offer_id)
    };
    let Some(o) = own else {
        return Err(AppError::NotFound(format!("offer {offer_id}")));
    };
    {
        let conn = lock(&state)?;
        store::withdraw_offer(&conn, &ctx.profile_id, &offer_id)?;
    }
    let cfg = env::config(&ctx.settings, ctx.network);
    if let Some(relay) = env::relay(&cfg, ctx.network)? {
        // Best effort: an offer that is not renewed leaves the relay within
        // three minutes anyway.
        let _ = relay
            .withdraw_offer(
                &o.signed,
                &relay_msg::sign_withdraw(&o.relay_key, &o.id),
                &o.relay_key,
            )
            .await;
    }
    Ok(())
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_my_offers(state: State<'_, AppState>) -> Result<Vec<OfferRow>, AppError> {
    let ctx = draft_ctx::load_ctx(&state)?;
    let conn = lock(&state)?;
    let t = now();
    Ok(store::offers(&conn, &ctx.profile_id)?
        .iter()
        .filter(|o| o.network == ctx.network.as_str() && o.live)
        .map(|o| {
            let live = o.live
                && o.last_error.is_none()
                && o.last_published_at.is_some_and(|p| p + 180 > t);
            offer_row(&o.signed, true, live)
        })
        .collect())
}

// ------------------------------------------------------------------ trades

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub buyer_fund_deadline: u64,
    pub seller_fund_deadline: u64,
    pub reveal_deadline: u64,
    pub hns_refund_time: u64,
    pub btc_refund_time: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockOut {
    pub txid: String,
    pub vout: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeRow {
    pub trade_id: String,
    pub terms_id: Option<String>,
    pub role: Role,
    pub maker: bool,
    pub offer_id: String,
    pub hns_amount: String,
    pub btc_sats: u64,
    pub price: String,
    pub status: Status,
    pub negotiating: bool,
    pub declined: Option<String>,
    pub timeline: Option<Timeline>,
    pub hns_lock: Option<LockOut>,
    pub btc_lock: Option<LockOut>,
    pub watchtower: String,
    pub last_error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

pub fn trade_row(t: &Trade) -> TradeRow {
    let status = t.status.clone().unwrap_or(match (&t.terms, t.role) {
        (Some(x), _) => Status::AwaitingBuyerCommit {
            deadline: x.buyer_fund_deadline,
        },
        // Negotiating: the take ends after the Buyer's funding window.
        (None, _) => Status::AwaitingBuyerCommit {
            // A stored network this build does not know has no policy: the
            // deadline reads as the creation time, i.e. already passed.
            deadline: t.created_at
                + Network::from_str_opt(&t.network)
                    .map(|n| env::policy(n).buyer_fund_window)
                    .unwrap_or(0),
        },
    });
    TradeRow {
        trade_id: t.trade_id.clone(),
        terms_id: t.terms.as_ref().map(|x| x.id()),
        role: t.role,
        maker: t.maker,
        offer_id: t.offer.offer.id(),
        hns_amount: hns_text(t.hns_amount),
        btc_sats: t.btc_sats,
        price: price::format_sats_per_hns(t.price_msat),
        // A trade that ended without terms (cancelled or expired while the
        // two sides were still agreeing) is no longer negotiating: its card
        // must not say so under "Trade expired" (QA 2026-11-10, Q6).
        negotiating: t.terms.is_none() && !status.is_terminal(),
        status,
        declined: t.declined.clone(),
        timeline: t.terms.as_ref().map(|x| Timeline {
            buyer_fund_deadline: x.buyer_fund_deadline,
            seller_fund_deadline: x.seller_fund_deadline,
            reveal_deadline: x.reveal_deadline,
            // What the card shows: when the lock really opens.
            hns_refund_time: x.hns_refund_opens(),
            btc_refund_time: x.btc_refund_time,
        }),
        hns_lock: t.hns_lock.as_ref().map(|(txid, vout)| LockOut {
            txid: txid.clone(),
            vout: *vout,
        }),
        btc_lock: t
            .holdings
            .as_ref()
            .and_then(|h| h.btc_lock.as_ref())
            .map(|l| LockOut {
                txid: l.txid.clone(),
                vout: l.vout,
            }),
        watchtower: t.watchtower.clone(),
        last_error: t.last_error.clone(),
        created_at: t.created_at,
        updated_at: t.updated_at,
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_list_trades(state: State<'_, AppState>) -> Result<Vec<TradeRow>, AppError> {
    let ctx = draft_ctx::load_ctx(&state)?;
    let conn = lock(&state)?;
    Ok(store::trades(&conn, &ctx.profile_id)?
        .iter()
        .filter(|t| t.network == ctx.network.as_str())
        .map(trade_row)
        .collect())
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_take_offer(
    state: State<'_, AppState>,
    offer_id: String,
    hns_amount: String,
) -> Result<TradeRow, AppError> {
    let ctx = trading_ctx(&state)?;
    let amount = parse_hns(&hns_amount)?;
    let cfg = env::config(&ctx.settings, ctx.network);
    let relay = env::relay(&cfg, ctx.network)?
        .ok_or_else(|| AppError::InvalidInput(env::NO_MEETING_POINT.into()))?;
    let t = now();
    let (list, _) = relay.offers(ctx.network.as_str(), t).await?;
    let Some(offer) = list.into_iter().find(|o| o.offer.id() == offer_id) else {
        return Err(AppError::NotFound(
            "the offer is no longer on the relay".into(),
        ));
    };
    let acct = {
        let conn = lock(&state)?;
        if store::offers(&conn, &ctx.profile_id)?
            .iter()
            .any(|o| o.id == offer_id)
        {
            return Err(AppError::InvalidInput("this is your own offer".into()));
        }
        account_of(&conn, &ctx)?
    };
    let f = &offer.offer;
    if amount < f.min_hns || amount > f.max_hns {
        return Err(AppError::InvalidInput(
            "the amount is outside the offer's range".into(),
        ));
    }
    let btc = price::btc_for(amount, f.price_msat_per_hns)?;
    let p = env::policy(ctx.network);
    if btc < p.min_btc_sats || btc > p.max_btc_sats {
        return Err(AppError::InvalidInput(format!(
            "a trade must be worth {} to {} sats",
            p.min_btc_sats, p.max_btc_sats
        )));
    }
    let role = match f.side {
        Side::Sell => Role::Buyer,
        Side::Buy => Role::Seller,
    };
    let index = {
        let conn = lock(&state)?;
        store::take_trade_indexes(&conn, &ctx.profile_id, 1)?
    };
    let (hns_pk, btc_pk) = keys::trade_pubkeys(&acct.swap_xpub, index)?;
    let secret_nonce = (role == Role::Buyer).then(keys::new_secret_nonce);
    let hash = match &secret_nonce {
        // The Buyer's hash, from the phrase and a fresh nonce.
        Some(n) => Some(hex::encode(with_signer(&state, &ctx, |s| {
            Ok(keys::trade_keys(s.master()?, ctx.network, ctx.account, index)?.hash(n))
        })?)),
        None => None,
    };
    let req = TakeRequest {
        hns_amount: amount,
        hns_pubkey: hex::encode(hns_pk),
        btc_pubkey: hex::encode(btc_pk),
        hash,
        start: t,
    };
    let key = relay_msg::new_relay_key();
    let trade_id = relay_msg::random_hex16();
    let trade = Trade {
        profile_id: ctx.profile_id.clone(),
        trade_id: trade_id.clone(),
        network: ctx.network.as_str().to_owned(),
        role,
        maker: false,
        offer: offer.clone(),
        trade_index: index,
        relay_key: key,
        counterparty_key: Some(f.maker_relay_pubkey.clone()),
        hns_amount: amount,
        btc_sats: btc,
        price_msat: f.price_msat_per_hns,
        terms: None,
        holdings: None,
        status: None,
        declined: None,
        last_error: None,
        watchtower: "none".into(),
        hns_lock: None,
        request: Some(req.clone()),
        secret_nonce,
        terminal_at: None,
        finished: false,
        version: 0,
        created_at: t,
        updated_at: t,
    };
    {
        let conn = lock(&state)?;
        store::insert_trade(&conn, &trade)?;
    }
    let msg = Message {
        version: relay_msg::MSG_VERSION,
        network: ctx.network.as_str().to_owned(),
        offer_id: offer_id.clone(),
        trade_id: trade_id.clone(),
        from: relay_msg::relay_pubkey_hex(&key),
        to: f.maker_relay_pubkey.clone(),
        sent_at: t,
        body: Body::Take(req),
    }
    .sign(&key)?;
    relay.send(&msg, &key).await?;
    Ok(trade_row(&trade))
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_cancel_trade(
    state: State<'_, AppState>,
    trade_id: String,
) -> Result<TradeRow, AppError> {
    let ctx = draft_ctx::load_ctx(&state)?;
    let mut t = {
        let conn = lock(&state)?;
        store::trade(&conn, &ctx.profile_id, &trade_id)?
            .ok_or_else(|| AppError::NotFound(format!("trade {trade_id}")))?
    };
    if t.holdings.as_ref().is_some_and(|h| h.committed) {
        return Err(AppError::InvalidInput(
            "you have locked coins in this trade: it ends on the chains, by a claim or a refund"
                .into(),
        ));
    }
    let cfg = env::config(&ctx.settings, ctx.network);
    if let (Some(relay), Some(to)) = (env::relay(&cfg, ctx.network)?, t.counterparty_key.clone()) {
        let m = Message {
            version: relay_msg::MSG_VERSION,
            network: ctx.network.as_str().to_owned(),
            offer_id: t.offer.offer.id(),
            trade_id: t.trade_id.clone(),
            from: relay_msg::relay_pubkey_hex(&t.relay_key),
            to,
            sent_at: now(),
            body: Body::Decline {
                reason: "cancelled".into(),
            },
        }
        .sign(&t.relay_key)?;
        let _ = relay.send(&m, &t.relay_key).await;
    }
    // Our own cancel is not the other side's decline.
    t.status = Some(Status::Expired);
    t.terminal_at = Some(now());
    t.finished = true;
    let conn = lock(&state)?;
    if !store::update_trade(&conn, &mut t, now())? {
        return Err(AppError::InvalidInput(
            "the trade changed meanwhile: look again and retry".into(),
        ));
    }
    Ok(trade_row(&t))
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_refresh(state: State<'_, AppState>) -> Result<Vec<TradeRow>, AppError> {
    let ctx = draft_ctx::load_ctx(&state)?;
    let path = db_path(&state)?;
    let profile = ctx.profile_id.clone();
    // The step holds its own database connection across awaits, which is not
    // `Send`: run it on a thread of its own, as the sync does.
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AppError::Other(format!("runtime: {e}")))?;
        rt.block_on(env::sync_step(&path, &profile));
        Ok::<(), AppError>(())
    })
    .await
    .map_err(|e| AppError::Other(format!("exchange step: {e}")))??;
    exchange_list_trades(state).await
}

/// The Commit (spec §3.2): the Buyer's or the Seller's, by the trade's role.
#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn exchange_commit(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    trade_id: String,
) -> Result<TradeRow, AppError> {
    let ctx = trading_ctx(&state)?;
    let (t, acct) = {
        let conn = lock(&state)?;
        let t = store::trade(&conn, &ctx.profile_id, &trade_id)?
            .ok_or_else(|| AppError::NotFound(format!("trade {trade_id}")))?;
        (t, account_of(&conn, &ctx)?)
    };
    let Some(terms) = t.terms.clone() else {
        return Err(AppError::InvalidInput(
            "the terms of this trade are not agreed yet".into(),
        ));
    };
    if t.holdings.as_ref().is_some_and(|h| h.committed) {
        return Err(AppError::InvalidInput(
            "you already confirmed this trade".into(),
        ));
    }
    let p = env::policy(ctx.network);
    let srcs = built_sources(&ctx)?;
    match t.role {
        Role::Buyer => buyer_commit(&app, &state, &ctx, t, terms, &acct, &srcs, &p).await,
        Role::Seller => seller_commit(&app, &state, &ctx, t, terms, &srcs, &p).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn buyer_commit(
    app: &tauri::AppHandle,
    state: &State<'_, AppState>,
    ctx: &Ctx,
    mut t: Trade,
    terms: crate::noncustodial::exchange::terms::Terms,
    acct: &store::Account,
    srcs: &[Box<dyn BtcSource>],
    p: &crate::noncustodial::exchange::policy::Policy,
) -> Result<TradeRow, AppError> {
    let n = now();
    if n > terms.buyer_fund_deadline {
        return Err(AppError::InvalidInput(
            "the time to lock BTC for this trade is over".into(),
        ));
    }
    let coins = btc_wallet::coins(srcs[0].as_ref(), acct, ctx.network).await?;
    let picked = btc_rate(srcs).await?;
    let rate = picked.sat_per_vb;
    let hns_rate = draft_ctx::fee_rate(ctx, None)?;
    let (hns_to, change_to, refund_to) = {
        let conn = lock(state)?;
        let hns_to =
            crate::noncustodial::derivation::reserve_receive_address(&conn, &ctx.profile_id)?
                .address;
        let c = store::take_btc_index(&conn, &ctx.profile_id, 1)?;
        let r = store::take_btc_index(&conn, &ctx.profile_id, 0)?;
        (
            hns_to,
            btc_account::address_at(&acct.btc_xpub, ctx.network, 1, c)?,
            btc_account::address_at(&acct.btc_xpub, ctx.network, 0, r)?,
        )
    };
    let nonce = t
        .secret_nonce
        .ok_or_else(|| AppError::Other("this trade has no secret nonce".into()))?;
    let build = |s: &mut SignerSession| {
        commit::buyer_commit(&commit::BuyerCommitInput {
            master: s.master()?,
            account: ctx.account,
            trade_index: t.trade_index,
            secret_nonce: &nonce,
            terms: &terms,
            policy: p,
            btc_coins: &coins,
            btc_change_to: &change_to,
            btc_refund_to: &refund_to,
            btc_rate: rate,
            hns_rate,
            hns_to: &hns_to,
        })
    };
    // Built once to show, and checked to be the same when signed: the
    // signatures are deterministic (RFC 6979), so the rows describe exactly
    // the transactions that are stored and sent.
    let shown = with_signer(state, ctx, build)?;
    let lock_addr = btc_htlc::lock_address(ctx.network, &terms.btc_params()?).to_string();
    crate::commands::secure_confirm::confirm_rows(
        app,
        review::BUYER_TITLE,
        review::BUYER_MESSAGE,
        with_rate_note(review::buyer_rows(&shown, &terms, &lock_addr)?, picked),
    )
    .await?;
    let c = with_signer(state, ctx, build)?;
    if c.funding.compute_txid() != shown.funding.compute_txid() || c.hns_claim != shown.hns_claim {
        return Err(AppError::Other(
            "the trade changed between the confirmation and the signing".into(),
        ));
    }
    // Store first, then send (module docs).
    t.holdings = Some(c.holdings.clone());
    t.status = Some(Status::AwaitingBtcLock {
        confirmations: 0,
        needed: terms.btc_confirmations,
    });
    {
        let conn = lock(state)?;
        // The deadline again, after the time the window took (review L4).
        if now() > terms.buyer_fund_deadline {
            return Err(AppError::InvalidInput(
                "the deadline passed while the window was open: nothing was sent".into(),
            ));
        }
        // Only if no Commit is stored yet and nobody wrote the trade since it
        // was read (review C2, M1); otherwise nothing is sent.
        if !store::commit_trade(&conn, &mut t, n)? {
            return Err(AppError::InvalidInput(
                "nothing was sent: this trade was confirmed already, or changed while the window was open; look again".into(),
            ));
        }
    }
    let sent = runner::send_btc(srcs, &btc_htlc::encode_tx(&c.funding)).await;
    if let Err(e) = &sent {
        t.last_error = Some(format!(
            "Bitcoin: the BTC lock was not sent yet ({e}); it is sent again on every sync"
        ));
        let conn = lock(state)?;
        store::update_trade(&conn, &mut t, n)?;
    }
    announce(
        ctx,
        &t,
        &terms,
        Body::BtcLocked(LockAnnouncement {
            terms_id: terms.id(),
            txid: c.btc_lock.txid.clone(),
            vout: c.btc_lock.vout,
        }),
    )
    .await;
    Ok(trade_row(&t))
}

async fn announce(
    ctx: &Ctx,
    t: &Trade,
    terms: &crate::noncustodial::exchange::terms::Terms,
    body: Body,
) {
    let cfg = env::config(&ctx.settings, ctx.network);
    let Ok(Some(relay)) = env::relay(&cfg, ctx.network) else {
        return;
    };
    let to = match t.role {
        Role::Buyer => terms.seller_relay_pubkey.clone(),
        Role::Seller => terms.buyer_relay_pubkey.clone(),
    };
    let msg = Message {
        version: relay_msg::MSG_VERSION,
        network: ctx.network.as_str().to_owned(),
        offer_id: t.offer.offer.id(),
        trade_id: t.trade_id.clone(),
        from: relay_msg::relay_pubkey_hex(&t.relay_key),
        to,
        sent_at: now(),
        body,
    };
    if let Ok(e) = msg.sign(&t.relay_key) {
        // Best effort: the counterparty's step finds the lock on chain too.
        let _ = relay.send(&e, &t.relay_key).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn seller_commit(
    app: &tauri::AppHandle,
    state: &State<'_, AppState>,
    ctx: &Ctx,
    mut t: Trade,
    terms: crate::noncustodial::exchange::terms::Terms,
    srcs: &[Box<dyn BtcSource>],
    p: &crate::noncustodial::exchange::policy::Policy,
) -> Result<TradeRow, AppError> {
    let n = now();
    let holdings = t
        .holdings
        .clone()
        .ok_or_else(|| AppError::InvalidInput("the terms are not agreed yet".into()))?;
    let Some(btc_lock) = holdings.btc_lock.clone() else {
        return Err(AppError::InvalidInput(
            "the buyer's BTC is not locked yet".into(),
        ));
    };
    // R9: the BTC lock checked now, by proof of work, every source agreeing.
    let view = btc_observe::observe(srcs, &holdings).await?;
    engine::seller_may_commit(&holdings, Some(&view), n)
        .map_err(|e| AppError::InvalidInput(e.into()))?;
    if let Some(drift) = p.max_clock_drift {
        let info = ctx.node.get_blockchain_info().await?;
        let mtp = info
            .mediantime
            .ok_or_else(|| AppError::Rpc("the node did not report its median time".into()))?;
        if mtp > n + drift || n > mtp + drift {
            return Err(AppError::InvalidInput(
                "this computer's clock and the HNS chain disagree by hours: fix the clock before trading".into(),
            ));
        }
    }
    let lock_addr = hns_htlc::lock_address(ctx.network, &terms.hns_params()?)?;
    let rate = draft_ctx::fee_rate(ctx, None)?;
    let plan = crate::noncustodial::send::build_send_plan(
        ctx.network,
        ctx.account,
        &ctx.funding,
        &lock_addr,
        terms.hns_amount,
        &ctx.change_address,
        rate,
        false,
    )?;
    let in_total: u64 = plan.inputs.iter().map(|i| i.value).sum();
    let out_total: u64 = plan.outputs.iter().map(|o| o.value).sum();
    let funding_fee = in_total - out_total;
    let picked = btc_rate(srcs).await?;
    let btc_rate_now = picked.sat_per_vb;
    let refund_to = {
        let conn = lock(state)?;
        crate::noncustodial::derivation::reserve_receive_address(&conn, &ctx.profile_id)?.address
    };
    let acct = {
        let conn = lock(state)?;
        account_of(&conn, ctx)?
    };
    let claim_to = {
        let conn = lock(state)?;
        let i = store::take_btc_index(&conn, &ctx.profile_id, 0)?;
        btc_account::address_at(&acct.btc_xpub, ctx.network, 0, i)?
    };
    let build = |s: &mut SignerSession| -> Result<(commit::SellerCommit, String), AppError> {
        let (hex_tx, _txid) = crate::noncustodial::actions::sign_plan(s, &plan)?;
        let raw = hex::decode(&hex_tx)
            .map_err(|_| AppError::Other("signed funding is not hex".into()))?;
        let funding = crate::noncustodial::tx::Transaction::decode(&raw)?;
        let c = commit::seller_commit(&commit::SellerCommitInput {
            master: s.master()?,
            account: ctx.account,
            trade_index: t.trade_index,
            terms: &terms,
            policy: p,
            hns_funding: &funding,
            btc_lock: &btc_lock,
            hns_rate: rate,
            hns_refund_to: &refund_to,
            btc_rate: btc_rate_now,
            btc_claim_to: &claim_to,
        })?;
        Ok((c, hex_tx))
    };
    let (shown, _) = with_signer(state, ctx, build)?;
    crate::commands::secure_confirm::confirm_rows(
        app,
        review::SELLER_TITLE,
        review::SELLER_MESSAGE,
        with_rate_note(
            review::seller_rows(&shown, &terms, funding_fee, &lock_addr)?,
            picked,
        ),
    )
    .await?;
    let (c, funding_hex) = with_signer(state, ctx, build)?;
    if c.hns_lock != shown.hns_lock || c.btc_claims != shown.btc_claims {
        return Err(AppError::Other(
            "the trade changed between the confirmation and the signing".into(),
        ));
    }
    // Store first, then send.
    t.holdings = Some(c.holdings.clone());
    t.hns_lock = Some((c.hns_lock.txid.clone(), c.hns_lock.vout));
    t.status = Some(Status::AwaitingHnsLock {
        confirmations: 0,
        needed: terms.hns_confirmations,
    });
    {
        let conn = lock(state)?;
        // The deadline again, after the time the window took (review L4).
        if now() > terms.seller_fund_deadline {
            return Err(AppError::InvalidInput(
                "the deadline passed while the window was open: nothing was sent".into(),
            ));
        }
        // Only if no Commit is stored yet and nobody wrote the trade since it
        // was read (review C2, M1); otherwise nothing is sent.
        if !store::commit_trade(&conn, &mut t, n)? {
            return Err(AppError::InvalidInput(
                "nothing was sent: this trade was confirmed already, or changed while the window was open; look again".into(),
            ));
        }
    }
    let hns_io = runner::HnsIo {
        node: &ctx.node,
        settings: &ctx.settings,
        network: ctx.network,
    };
    match runner::send_hns(&hns_io, &funding_hex).await {
        Ok(true) => {}
        Ok(false) => {
            t.last_error = Some(
                "HNS: the node did not take the lock yet; it is sent again on every sync".into(),
            )
        }
        Err(e) => {
            t.last_error = Some(format!(
                "HNS: the lock was not sent yet ({e}); it is sent again on every sync"
            ))
        }
    }
    {
        let conn = lock(state)?;
        store::update_trade(&conn, &mut t, n)?;
    }
    announce(
        ctx,
        &t,
        &terms,
        Body::HnsLocked(LockAnnouncement {
            terms_id: terms.id(),
            txid: c.hns_lock.txid.clone(),
            vout: c.hns_lock.vout,
        }),
    )
    .await;
    Ok(trade_row(&t))
}
