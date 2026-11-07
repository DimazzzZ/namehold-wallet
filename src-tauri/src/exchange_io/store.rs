//! The exchange's rows in the wallet database (`sql/044_exchange.sql`).
//! Typed structs in and out; JSON columns hold the pure layer's own types.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;
use crate::noncustodial::exchange::engine::{Holdings, Role, Status};
use crate::noncustodial::exchange::relay_msg::{Side, SignedOffer};
use crate::noncustodial::exchange::terms::Terms;
use crate::noncustodial::hd::ExtendedPubKey;

fn db(e: rusqlite::Error) -> AppError {
    AppError::Db(e)
}

fn corrupt(what: &str) -> AppError {
    AppError::Other(format!("corrupted exchange row: {what}"))
}

#[derive(Debug, Clone)]
pub struct Account {
    pub profile_id: String,
    pub network: String,
    pub btc_xpub: ExtendedPubKey,
    pub swap_xpub: ExtendedPubKey,
    pub account: u32,
    pub next_trade_index: u32,
    pub next_btc_receive: u32,
    pub next_btc_change: u32,
}

pub fn xpub_hex(x: &ExtendedPubKey) -> String {
    let mut v = x.compressed_pubkey().to_vec();
    v.extend_from_slice(&x.chain_code);
    hex::encode(v)
}

fn xpub_from_hex(s: &str) -> Result<ExtendedPubKey, AppError> {
    let b = hex::decode(s).map_err(|_| corrupt("xpub hex"))?;
    if b.len() != 65 {
        return Err(corrupt("xpub length"));
    }
    let pk: [u8; 33] = b[..33].try_into().expect("length checked");
    let cc: [u8; 32] = b[33..].try_into().expect("length checked");
    ExtendedPubKey::from_parts(&pk, &cc)
}

pub fn insert_account(conn: &Connection, a: &Account, now: u64) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO exchange_accounts (profile_id, network, btc_xpub_hex, swap_xpub_hex, account,
            next_trade_index, next_btc_receive, next_btc_change, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0, ?6)
         ON CONFLICT(profile_id) DO NOTHING",
        params![
            a.profile_id,
            a.network,
            xpub_hex(&a.btc_xpub),
            xpub_hex(&a.swap_xpub),
            a.account,
            now as i64
        ],
    )
    .map_err(db)?;
    Ok(())
}

pub fn account(conn: &Connection, profile_id: &str) -> Result<Option<Account>, AppError> {
    let row = conn
        .query_row(
            "SELECT network, btc_xpub_hex, swap_xpub_hex, account, next_trade_index, next_btc_receive, next_btc_change
             FROM exchange_accounts WHERE profile_id = ?1",
            [profile_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(db)?;
    let Some((network, b, s, acct, t, rcv, chg)) = row else {
        return Ok(None);
    };
    let u = |v: i64, w: &str| u32::try_from(v).map_err(|_| corrupt(w));
    Ok(Some(Account {
        profile_id: profile_id.to_owned(),
        network,
        btc_xpub: xpub_from_hex(&b)?,
        swap_xpub: xpub_from_hex(&s)?,
        account: u(acct, "account")?,
        next_trade_index: u(t, "trade index")?,
        next_btc_receive: u(rcv, "receive index")?,
        next_btc_change: u(chg, "change index")?,
    }))
}

/// Hand out `n` trade indexes; returns the first.
pub fn take_trade_indexes(conn: &Connection, profile_id: &str, n: u32) -> Result<u32, AppError> {
    let tx = conn.unchecked_transaction().map_err(db)?;
    let first: i64 = tx
        .query_row(
            "SELECT next_trade_index FROM exchange_accounts WHERE profile_id = ?1",
            [profile_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    let next = first + i64::from(n);
    if next >= i64::from(crate::noncustodial::hd::HARDENED_OFFSET) {
        return Err(AppError::InvalidInput("no trade index left".into()));
    }
    tx.execute(
        "UPDATE exchange_accounts SET next_trade_index = ?2 WHERE profile_id = ?1",
        params![profile_id, next],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(first as u32)
}

/// Hand out the next BTC address index of `branch` (0 receive, 1 change).
pub fn take_btc_index(conn: &Connection, profile_id: &str, branch: u32) -> Result<u32, AppError> {
    let col = if branch == 0 {
        "next_btc_receive"
    } else {
        "next_btc_change"
    };
    let tx = conn.unchecked_transaction().map_err(db)?;
    let i: i64 = tx
        .query_row(
            &format!("SELECT {col} FROM exchange_accounts WHERE profile_id = ?1"),
            [profile_id],
            |r| r.get(0),
        )
        .map_err(db)?;
    tx.execute(
        &format!("UPDATE exchange_accounts SET {col} = {col} + 1 WHERE profile_id = ?1"),
        [profile_id],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    u32::try_from(i).map_err(|_| corrupt("btc index"))
}

// ------------------------------------------------------------------ offers

#[derive(Debug, Clone)]
pub struct OwnOffer {
    pub id: String,
    pub network: String,
    pub side: Side,
    pub signed: SignedOffer,
    pub relay_key: secp256k1::SecretKey,
    pub live: bool,
    pub last_published_at: Option<u64>,
    pub last_error: Option<String>,
}

pub fn insert_offer(
    conn: &Connection,
    profile_id: &str,
    o: &OwnOffer,
    now: u64,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO exchange_offers (id, profile_id, network, side, signed_json, relay_key_hex, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'live', ?7)",
        params![
            o.id,
            profile_id,
            o.network,
            side_str(o.side),
            serde_json::to_string(&o.signed).expect("offers serialize"),
            hex::encode(o.relay_key.secret_bytes()),
            now as i64
        ],
    )
    .map_err(db)?;
    Ok(())
}

pub fn side_str(s: Side) -> &'static str {
    match s {
        Side::Sell => "sell",
        Side::Buy => "buy",
    }
}

fn key_from_hex(s: &str) -> Result<secp256k1::SecretKey, AppError> {
    let mut b = [0u8; 32];
    hex::decode_to_slice(s, &mut b).map_err(|_| corrupt("relay key"))?;
    secp256k1::SecretKey::from_byte_array(b).map_err(|_| corrupt("relay key"))
}

pub fn offers(conn: &Connection, profile_id: &str) -> Result<Vec<OwnOffer>, AppError> {
    let mut st = conn
        .prepare(
            "SELECT id, network, side, signed_json, relay_key_hex, state, last_published_at, last_error
             FROM exchange_offers WHERE profile_id = ?1 ORDER BY created_at DESC",
        )
        .map_err(db)?;
    let rows = st
        .query_map([profile_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(db)?;
    let mut out = Vec::new();
    for r in rows {
        let (id, network, side, json, key, state, published, err) = r.map_err(db)?;
        let signed: SignedOffer = serde_json::from_str(&json).map_err(|_| corrupt("offer json"))?;
        out.push(OwnOffer {
            id,
            network,
            side: if side == "sell" {
                Side::Sell
            } else {
                Side::Buy
            },
            signed,
            relay_key: key_from_hex(&key)?,
            live: state == "live",
            last_published_at: published.map(|v| v as u64),
            last_error: err,
        });
    }
    Ok(out)
}

pub fn set_offer_published(
    conn: &Connection,
    id: &str,
    at: u64,
    err: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE exchange_offers SET last_published_at = CASE WHEN ?3 IS NULL THEN ?2 ELSE last_published_at END,
            last_error = ?3 WHERE id = ?1",
        params![id, at as i64, err],
    )
    .map_err(db)?;
    Ok(())
}

pub fn withdraw_offer(conn: &Connection, profile_id: &str, id: &str) -> Result<bool, AppError> {
    let n = conn
        .execute(
            "UPDATE exchange_offers SET state = 'withdrawn' WHERE id = ?1 AND profile_id = ?2",
            params![id, profile_id],
        )
        .map_err(db)?;
    Ok(n > 0)
}

pub fn insert_hashes(
    conn: &Connection,
    profile_id: &str,
    hashes: &[(u32, [u8; 32], [u8; 32])],
) -> Result<(), AppError> {
    for (i, h, n) in hashes {
        conn.execute(
            "INSERT OR IGNORE INTO exchange_hash_pool (profile_id, trade_index, hash_hex, nonce_hex) VALUES (?1, ?2, ?3, ?4)",
            params![profile_id, i, hex::encode(h), hex::encode(n)],
        )
        .map_err(db)?;
    }
    Ok(())
}

/// A pooled hash: `(trade index, hash, nonce)`.
pub type PooledHash = (u32, [u8; 32], [u8; 32]);

/// The next unused pooled hash, marked used.
pub fn take_hash(conn: &Connection, profile_id: &str) -> Result<Option<PooledHash>, AppError> {
    let tx = conn.unchecked_transaction().map_err(db)?;
    let row = tx
        .query_row(
            "SELECT trade_index, hash_hex, nonce_hex FROM exchange_hash_pool WHERE profile_id = ?1 AND used = 0
             ORDER BY trade_index LIMIT 1",
            [profile_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(db)?;
    let Some((i, h, n)) = row else {
        return Ok(None);
    };
    tx.execute(
        "UPDATE exchange_hash_pool SET used = 1 WHERE profile_id = ?1 AND trade_index = ?2",
        params![profile_id, i],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    let mut b = [0u8; 32];
    hex::decode_to_slice(&h, &mut b).map_err(|_| corrupt("pooled hash"))?;
    let mut nb = [0u8; 32];
    hex::decode_to_slice(&n, &mut nb).map_err(|_| corrupt("pooled nonce"))?;
    Ok(Some((
        u32::try_from(i).map_err(|_| corrupt("pool index"))?,
        b,
        nb,
    )))
}

// ------------------------------------------------------------------ trades

#[derive(Debug, Clone)]
pub struct Trade {
    pub profile_id: String,
    pub trade_id: String,
    pub network: String,
    pub role: Role,
    pub maker: bool,
    pub offer: SignedOffer,
    pub trade_index: u32,
    pub relay_key: secp256k1::SecretKey,
    pub counterparty_key: Option<String>,
    pub hns_amount: u64,
    pub btc_sats: u64,
    pub price_msat: u64,
    pub terms: Option<Terms>,
    pub holdings: Option<Holdings>,
    pub status: Option<Status>,
    pub declined: Option<String>,
    pub last_error: Option<String>,
    pub watchtower: String,
    pub hns_lock: Option<(String, u32)>,
    pub request: Option<crate::noncustodial::exchange::relay_msg::TakeRequest>,
    /// The Buyer's secret nonce (`keys` module docs); `None` for the Seller.
    pub secret_nonce: Option<[u8; 32]>,
    pub terminal_at: Option<u64>,
    pub finished: bool,
    /// The row's version when it was read (`update_trade` compares it).
    pub version: i64,
    pub created_at: u64,
    pub updated_at: u64,
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::Buyer => "buyer",
        Role::Seller => "seller",
    }
}

pub fn insert_trade(conn: &Connection, t: &Trade) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO exchange_trades (profile_id, trade_id, network, role, maker, offer_id, offer_json, trade_index,
            relay_key_hex, counterparty_key, hns_amount, btc_sats, price_msat, request_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?15, ?14, ?14)",
        params![
            t.profile_id,
            t.trade_id,
            t.network,
            role_str(t.role),
            t.maker as i64,
            t.offer.offer.id(),
            serde_json::to_string(&t.offer).expect("offers serialize"),
            t.trade_index,
            hex::encode(t.relay_key.secret_bytes()),
            t.counterparty_key,
            t.hns_amount as i64,
            t.btc_sats as i64,
            t.price_msat as i64,
            t.created_at as i64,
            t.request.as_ref().map(|r| serde_json::to_string(r).expect("requests serialize")),
        ],
    )
    .map_err(db)?;
    Ok(())
}

/// Store everything that changes after a trade's start, only if the row
/// is still the version `t` was read at (and never undoing a Commit).
/// `Ok(false)` when someone else wrote it meanwhile: the caller drops its
/// change, and the next step starts again from the stored row. On success
/// `t.version` is the new version.
pub fn update_trade(conn: &Connection, t: &mut Trade, now: u64) -> Result<bool, AppError> {
    write_trade(conn, t, now, false)
}

/// Store this party's Commit: like [`update_trade`], and only if no Commit
/// is stored yet. `Ok(false)` means nothing was stored, so the caller must
/// send nothing (review C2, M1: a Commit racing the background step or a
/// second Commit of the same trade).
pub fn commit_trade(conn: &Connection, t: &mut Trade, now: u64) -> Result<bool, AppError> {
    write_trade(conn, t, now, true)
}

fn write_trade(
    conn: &Connection,
    t: &mut Trade,
    now: u64,
    committing: bool,
) -> Result<bool, AppError> {
    let committed = t.holdings.as_ref().is_some_and(|h| h.committed);
    if committing && !committed {
        return Err(AppError::Other(
            "a Commit without committed holdings".into(),
        ));
    }
    let n = conn
        .execute(
            "UPDATE exchange_trades SET counterparty_key = ?3, terms_json = ?4, terms_id = ?5, holdings_json = ?6,
                status_json = ?7, declined = ?8, last_error = ?9, watchtower = ?10, hns_lock_txid = ?11,
                hns_lock_vout = ?12, finished = ?13, updated_at = ?14, terminal_at = ?15,
                secret_nonce_hex = ?16, committed = MAX(committed, ?17), version = version + 1
             WHERE profile_id = ?1 AND trade_id = ?2 AND version = ?18
               AND (?19 = 0 OR committed = 0)
               AND (committed = 0 OR ?17 = 1)",
            params![
                t.profile_id,
                t.trade_id,
                t.counterparty_key,
                t.terms.as_ref().map(|x| serde_json::to_string(x).expect("terms serialize")),
                t.terms.as_ref().map(|x| x.id()),
                t.holdings.as_ref().map(|x| serde_json::to_string(x).expect("holdings serialize")),
                t.status.as_ref().map(|x| serde_json::to_string(x).expect("status serializes")),
                t.declined,
                t.last_error,
                t.watchtower,
                t.hns_lock.as_ref().map(|x| x.0.clone()),
                t.hns_lock.as_ref().map(|x| x.1),
                t.finished as i64,
                now as i64,
                t.terminal_at.map(|v| v as i64),
                t.secret_nonce.map(hex::encode),
                committed as i64,
                t.version,
                committing as i64,
            ],
        )
        .map_err(db)?;
    if n == 1 {
        t.version += 1;
    }
    Ok(n == 1)
}

const TRADE_COLS: &str = "profile_id, trade_id, network, role, maker, offer_json, trade_index, relay_key_hex,
    counterparty_key, hns_amount, btc_sats, price_msat, terms_json, holdings_json, status_json, declined,
    last_error, watchtower, hns_lock_txid, hns_lock_vout, finished, created_at, updated_at, request_json, terminal_at, secret_nonce_hex, version";

fn trade_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Trade, AppError>> {
    let role: String = r.get(3)?;
    let offer_json: String = r.get(5)?;
    let key: String = r.get(7)?;
    let terms: Option<String> = r.get(12)?;
    let holdings: Option<String> = r.get(13)?;
    let status: Option<String> = r.get(14)?;
    let txid: Option<String> = r.get(18)?;
    let vout: Option<i64> = r.get(19)?;
    let base = (
        r.get::<_, String>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, String>(2)?,
        r.get::<_, i64>(4)?,
        r.get::<_, i64>(6)?,
        r.get::<_, Option<String>>(8)?,
        r.get::<_, i64>(9)?,
        r.get::<_, i64>(10)?,
        r.get::<_, i64>(11)?,
        r.get::<_, Option<String>>(15)?,
        r.get::<_, Option<String>>(16)?,
        r.get::<_, String>(17)?,
        r.get::<_, i64>(20)?,
        r.get::<_, i64>(21)?,
        r.get::<_, i64>(22)?,
        r.get::<_, Option<String>>(23)?,
        r.get::<_, Option<i64>>(24)?,
        r.get::<_, Option<String>>(25)?,
        r.get::<_, i64>(26)?,
    );
    Ok((|| {
        let (
            profile_id,
            trade_id,
            network,
            maker,
            idx,
            cp,
            hns,
            btc,
            price,
            declined,
            err,
            wt,
            fin,
            c,
            u,
            req,
            term,
            nonce,
            version,
        ) = base;
        Ok(Trade {
            profile_id,
            trade_id,
            network,
            role: match role.as_str() {
                "buyer" => Role::Buyer,
                "seller" => Role::Seller,
                _ => return Err(corrupt("role")),
            },
            maker: maker != 0,
            offer: serde_json::from_str(&offer_json).map_err(|_| corrupt("offer json"))?,
            trade_index: u32::try_from(idx).map_err(|_| corrupt("trade index"))?,
            relay_key: key_from_hex(&key)?,
            counterparty_key: cp,
            hns_amount: hns as u64,
            btc_sats: btc as u64,
            price_msat: price as u64,
            terms: terms
                .map(|j| Terms::parse(&j))
                .transpose()
                .map_err(|_| corrupt("terms json"))?,
            holdings: holdings
                .map(|j| serde_json::from_str(&j))
                .transpose()
                .map_err(|_| corrupt("holdings json"))?,
            status: status
                .map(|j| serde_json::from_str(&j))
                .transpose()
                .map_err(|_| corrupt("status json"))?,
            declined,
            last_error: err,
            watchtower: wt,
            hns_lock: match (txid, vout) {
                (Some(t), Some(v)) => Some((t, u32::try_from(v).map_err(|_| corrupt("vout"))?)),
                _ => None,
            },
            request: req
                .map(|j| serde_json::from_str(&j))
                .transpose()
                .map_err(|_| corrupt("request json"))?,
            terminal_at: term.map(|v| v as u64),
            secret_nonce: nonce
                .map(|h| {
                    let mut b = [0u8; 32];
                    hex::decode_to_slice(&h, &mut b).map(|_| b)
                })
                .transpose()
                .map_err(|_| corrupt("secret nonce"))?,
            finished: fin != 0,
            version,
            created_at: c as u64,
            updated_at: u as u64,
        })
    })())
}

pub fn trades(conn: &Connection, profile_id: &str) -> Result<Vec<Trade>, AppError> {
    let mut st = conn
        .prepare(&format!(
            "SELECT {TRADE_COLS} FROM exchange_trades WHERE profile_id = ?1 ORDER BY created_at DESC"
        ))
        .map_err(db)?;
    let rows = st.query_map([profile_id], trade_from_row).map_err(db)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(db)??);
    }
    Ok(out)
}

pub fn trade(
    conn: &Connection,
    profile_id: &str,
    trade_id: &str,
) -> Result<Option<Trade>, AppError> {
    let mut st = conn
        .prepare(&format!(
            "SELECT {TRADE_COLS} FROM exchange_trades WHERE profile_id = ?1 AND trade_id = ?2"
        ))
        .map_err(db)?;
    let mut rows = st
        .query_map(params![profile_id, trade_id], trade_from_row)
        .map_err(db)?;
    match rows.next() {
        None => Ok(None),
        Some(r) => Ok(Some(r.map_err(db)??)),
    }
}

// ------------------------------------------------------------------- inbox

pub fn inbox_cursor(conn: &Connection, relay_pubkey: &str) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT cursor FROM exchange_inbox WHERE relay_pubkey = ?1",
        [relay_pubkey],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()
    .map_err(db)
    .map(Option::flatten)
}

pub fn set_inbox_cursor(
    conn: &Connection,
    profile_id: &str,
    relay_pubkey: &str,
    cursor: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO exchange_inbox (relay_pubkey, profile_id, cursor) VALUES (?1, ?2, ?3)
         ON CONFLICT(relay_pubkey) DO UPDATE SET cursor = excluded.cursor",
        params![relay_pubkey, profile_id, cursor],
    )
    .map_err(db)?;
    Ok(())
}
