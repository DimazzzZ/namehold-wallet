//! Local wallet sync engine: turns node reads into the chain cache that backs
//! balances, coin selection, and the Names UI — all without a custodial wallet.
//!
//! Flow per profile:
//!   1. Derive the receive/change address window (`derivation`).
//!   2. For each address, `getcoinsbyaddress` on the node.
//!   3. Upsert the returned coins into `tracked_utxos`, classifying each by its
//!      covenant into a `spend_class` so coin selection never accidentally
//!      spends name-locked value.
//!   4. Mark UTXOs no longer returned by the node as spent.
//!   5. Advance the `sync_cursors` height.
//!
//! Covenant classification is verified against hsd `lib/covenants/rules.js`
//! covenant type table.

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::noncustodial::rpc::{NodeCoin, NodeCovenant};

// --- hsd covenant types (lib/covenants/rules.js `types`) -------------------
pub const COV_NONE: u8 = 0;
pub const COV_CLAIM: u8 = 1;
pub const COV_OPEN: u8 = 2;
pub const COV_BID: u8 = 3;
pub const COV_REVEAL: u8 = 4;
pub const COV_REDEEM: u8 = 5;
pub const COV_REGISTER: u8 = 6;
pub const COV_UPDATE: u8 = 7;
pub const COV_RENEW: u8 = 8;
pub const COV_TRANSFER: u8 = 9;
pub const COV_FINALIZE: u8 = 10;
pub const COV_REVOKE: u8 = 11;

/// How a tracked UTXO may be spent. Drives coin selection so liquid funds and
/// name-bound value are never conflated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendClass {
    /// Ordinary spendable HNS (no covenant / covenant type NONE).
    LiquidHns,
    /// Output carries a name covenant the wallet controls and can advance
    /// (OPEN/CLAIM/REVEAL/REDEEM/REGISTER/UPDATE/RENEW/TRANSFER/FINALIZE).
    NameControl,
    /// Value locked by an in-flight auction bid (BID) — not freely spendable
    /// until revealed/redeemed.
    NameLockup,
    /// A covenant we don't model (e.g. REVOKE) — treated as unspendable.
    Unsupported,
}

impl SpendClass {
    /// The string stored in `tracked_utxos.spend_class` (matches the CHECK
    /// constraint in migration 007).
    pub fn as_str(self) -> &'static str {
        match self {
            SpendClass::LiquidHns => "liquid_hns",
            SpendClass::NameControl => "name_control",
            SpendClass::NameLockup => "name_lockup",
            SpendClass::Unsupported => "unsupported",
        }
    }
}

/// Classify a covenant type into a spend class.
pub fn classify_covenant(covenant_type: u8) -> SpendClass {
    match covenant_type {
        COV_NONE => SpendClass::LiquidHns,
        COV_BID => SpendClass::NameLockup,
        COV_CLAIM | COV_OPEN | COV_REVEAL | COV_REDEEM | COV_REGISTER | COV_UPDATE | COV_RENEW
        | COV_TRANSFER | COV_FINALIZE => SpendClass::NameControl,
        // REVOKE and any unknown future type: don't let coin selection touch it.
        _ => SpendClass::Unsupported,
    }
}

/// Effective covenant type of a coin (defaults to NONE when absent).
fn coin_covenant_type(coin: &NodeCoin) -> u8 {
    coin.covenant.as_ref().map(|c| c.kind).unwrap_or(COV_NONE)
}

/// Serialize a covenant to JSON for the `covenant_json` column, or `None` when
/// there's no covenant (type NONE).
fn covenant_json(covenant: &Option<NodeCovenant>) -> Option<String> {
    match covenant {
        Some(cov) if cov.kind != COV_NONE => Some(
            serde_json::json!({
                "type": cov.kind,
                "action": cov.action,
                "items": cov.items,
            })
            .to_string(),
        ),
        _ => None,
    }
}

/// Upsert a single node coin into `tracked_utxos` for a profile.
///
/// Idempotent on `(profile, txid, vout)`: re-syncing refreshes the height,
/// value, covenant and spend-class while clearing any stale `spent_by_txid`
/// (the node still reports it, so it is unspent).
pub fn upsert_utxo(conn: &Connection, profile_id: &str, coin: &NodeCoin) -> Result<(), AppError> {
    let cov_type = coin_covenant_type(coin);
    let spend_class = classify_covenant(cov_type).as_str();
    let cov_json = covenant_json(&coin.covenant);
    let address = coin
        .address
        .clone()
        .ok_or_else(|| AppError::InvalidInput("node coin missing address".to_string()))?;
    let script = coin.script.clone().unwrap_or_default();
    let coinbase = coin.coinbase.unwrap_or(false) as i64;

    conn.execute(
        "INSERT INTO tracked_utxos
            (txid, vout, wallet_profile_id, address, script_pubkey_hex,
             value_doos, height, coinbase, covenant_type, covenant_json,
             spend_class, spent_by_txid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)
         ON CONFLICT(wallet_profile_id, txid, vout) DO UPDATE SET
            address          = excluded.address,
            script_pubkey_hex= excluded.script_pubkey_hex,
            value_doos       = excluded.value_doos,
            height           = excluded.height,
            coinbase         = excluded.coinbase,
            covenant_type    = excluded.covenant_type,
            covenant_json    = excluded.covenant_json,
            spend_class      = excluded.spend_class,
            spent_by_txid    = NULL",
        params![
            coin.txid,
            coin.vout as i64,
            profile_id,
            address,
            script,
            coin.value,
            coin.height,
            coinbase,
            cov_type as i64,
            cov_json,
            spend_class,
        ],
    )?;
    Ok(())
}

/// Reconcile the tracked UTXO set for a profile against the live coins the node
/// reports. Any previously-tracked, still-unspent UTXO that is NOT in
/// `live_coins` is marked spent (we don't learn the spending txid here, so we
/// record a sentinel). Returns the number of UTXOs newly marked spent.
pub fn mark_missing_as_spent(
    conn: &Connection,
    profile_id: &str,
    live_coins: &[NodeCoin],
) -> Result<usize, AppError> {
    use std::collections::HashSet;
    let live: HashSet<(String, u32)> = live_coins
        .iter()
        .map(|c| (c.txid.clone(), c.vout))
        .collect();

    let mut stmt = conn.prepare(
        "SELECT txid, vout FROM tracked_utxos
         WHERE wallet_profile_id = ?1 AND spent_by_txid IS NULL",
    )?;
    let tracked: Vec<(String, i64)> = stmt
        .query_map([profile_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;

    let mut spent = 0usize;
    for (txid, vout) in tracked {
        if !live.contains(&(txid.clone(), vout as u32)) {
            conn.execute(
                "UPDATE tracked_utxos SET spent_by_txid = 'spent'
                 WHERE wallet_profile_id = ?1 AND txid = ?2 AND vout = ?3",
                params![profile_id, txid, vout],
            )?;
            spent += 1;
        }
    }
    Ok(spent)
}

/// Aggregate balance (in dollarydoos) for a profile, split by spend class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Balances {
    /// Freely spendable HNS.
    pub liquid: i64,
    /// Value bound to names the wallet controls.
    pub name_control: i64,
    /// Value locked in in-flight auction bids.
    pub name_lockup: i64,
}

impl Balances {
    pub fn total(&self) -> i64 {
        self.liquid + self.name_control + self.name_lockup
    }
}

/// Compute unspent balances for a profile from the chain cache.
pub fn compute_balances(conn: &Connection, profile_id: &str) -> Result<Balances, AppError> {
    let mut stmt = conn.prepare(
        "SELECT spend_class, COALESCE(SUM(value_doos), 0)
         FROM tracked_utxos
         WHERE wallet_profile_id = ?1 AND spent_by_txid IS NULL
         GROUP BY spend_class",
    )?;
    let rows = stmt.query_map([profile_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;

    let mut balances = Balances::default();
    for row in rows {
        let (class, sum) = row?;
        match class.as_str() {
            "liquid_hns" => balances.liquid = sum,
            "name_control" => balances.name_control = sum,
            "name_lockup" => balances.name_lockup = sum,
            _ => {} // unsupported is not counted as spendable balance
        }
    }
    Ok(balances)
}

/// Cache a transaction's decoded JSON for a profile (idempotent on txid).
pub fn cache_transaction(
    conn: &Connection,
    profile_id: &str,
    txid: &str,
    height: Option<i64>,
    time: Option<&str>,
    raw_json: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO wallet_transactions_cache
            (wallet_profile_id, txid, height, time, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(wallet_profile_id, txid) DO UPDATE SET
            height   = excluded.height,
            time     = excluded.time,
            raw_json = excluded.raw_json",
        params![profile_id, txid, height, time, raw_json],
    )?;
    Ok(())
}

/// Advance (or initialize) the sync cursor for a profile to `height`.
pub fn set_sync_cursor(conn: &Connection, profile_id: &str, height: i64) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO sync_cursors (wallet_profile_id, last_height, last_synced_at)
         VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(wallet_profile_id) DO UPDATE SET
            last_height    = excluded.last_height,
            last_synced_at = excluded.last_synced_at",
        params![profile_id, height],
    )?;
    Ok(())
}

/// Read the last synced height for a profile (0 if never synced).
pub fn get_sync_height(conn: &Connection, profile_id: &str) -> Result<i64, AppError> {
    let height: Option<i64> = conn
        .query_row(
            "SELECT last_height FROM sync_cursors WHERE wallet_profile_id = ?1",
            [profile_id],
            |row| row.get(0),
        )
        .ok();
    Ok(height.unwrap_or(0))
}

/// Mark a derived address as used at a given height (called when a coin is
/// found paying to it). Updates `used` and the first/last seen heights.
pub fn mark_address_used(
    conn: &Connection,
    profile_id: &str,
    address: &str,
    height: Option<i64>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE derived_addresses SET
            used = 1,
            first_seen_height = COALESCE(first_seen_height, ?3),
            last_seen_height = ?3
         WHERE wallet_profile_id = ?1 AND address = ?2",
        params![profile_id, address, height],
    )?;
    Ok(())
}

/// Upsert a name's on-chain state into `tracked_name_states` from a
/// `getnameinfo` JSON payload.
///
/// hsd `getnameinfo` returns `{ "info": { name, nameHash, state, height,
/// renewal, owner: { hash, index }, ... } }`, or `{ "info": null }` for an
/// unseen name. The caller passes the full RPC result; we read `info`.
pub fn upsert_name_state(
    conn: &Connection,
    profile_id: &str,
    name: &str,
    name_info: &serde_json::Value,
) -> Result<(), AppError> {
    let info = name_info.get("info");
    // An unopened/unknown name has null info — record a minimal row so the UI
    // can still show the name with an UNKNOWN/CLOSED-less state.
    let info = match info {
        Some(v) if !v.is_null() => v,
        _ => {
            conn.execute(
                "INSERT INTO tracked_name_states
                    (wallet_profile_id, name, name_hash_hex, state, raw_json)
                 VALUES (?1, ?2, '', 'UNKNOWN', ?3)
                 ON CONFLICT(wallet_profile_id, name) DO UPDATE SET
                    state    = 'UNKNOWN',
                    raw_json = excluded.raw_json,
                    updated_at = datetime('now')",
                params![profile_id, name, name_info.to_string()],
            )?;
            return Ok(());
        }
    };

    let name_hash = info
        .get("nameHash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let state = info
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN")
        .to_string();
    let height = info.get("height").and_then(|v| v.as_i64());
    let renewal_height = info.get("renewal").and_then(|v| v.as_i64());
    let transfer_height = info.get("transfer").and_then(|v| v.as_i64());
    let renewals = info.get("renewals").and_then(|v| v.as_i64());
    let weak = info.get("weak").and_then(|v| v.as_bool()).map(|b| b as i64);
    let owner = info.get("owner");
    let owner_txid = owner
        .and_then(|o| o.get("hash"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let owner_vout = owner.and_then(|o| o.get("index")).and_then(|v| v.as_i64());

    conn.execute(
        "INSERT INTO tracked_name_states
            (wallet_profile_id, name, name_hash_hex, state, owner_txid,
             owner_vout, height, renewal_height, transfer_height, renewals,
             weak, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(wallet_profile_id, name) DO UPDATE SET
            name_hash_hex   = excluded.name_hash_hex,
            state           = excluded.state,
            owner_txid      = excluded.owner_txid,
            owner_vout      = excluded.owner_vout,
            height          = excluded.height,
            renewal_height  = excluded.renewal_height,
            transfer_height = excluded.transfer_height,
            renewals        = excluded.renewals,
            weak            = excluded.weak,
            raw_json        = excluded.raw_json,
            updated_at      = datetime('now')",
        params![
            profile_id,
            name,
            name_hash,
            state,
            owner_txid,
            owner_vout,
            height,
            renewal_height,
            transfer_height,
            renewals,
            weak,
            name_info.to_string(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::rpc::{NodeCoin, NodeCovenant};

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(include_str!("../sql/006_noncustodial_wallet_profiles.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/007_noncustodial_chain_cache.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/008_noncustodial_name_state.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
             VALUES ('p1', 'Test', 'watch_only_xpub', 'mainnet', 'xpubPLACEHOLDER')",
            [],
        )
        .unwrap();
        conn
    }

    fn coin(txid: &str, vout: u32, value: i64, covenant: Option<NodeCovenant>) -> NodeCoin {
        NodeCoin {
            txid: txid.to_string(),
            vout,
            value,
            script: Some("0014abcd".to_string()),
            address: Some("hs1qexample".to_string()),
            height: Some(100),
            confirmations: Some(6),
            coinbase: Some(false),
            covenant,
        }
    }

    fn cov(kind: u8) -> NodeCovenant {
        NodeCovenant {
            kind,
            action: None,
            items: vec![],
        }
    }

    #[test]
    fn covenant_classification_matches_spec() {
        assert_eq!(classify_covenant(COV_NONE), SpendClass::LiquidHns);
        assert_eq!(classify_covenant(COV_BID), SpendClass::NameLockup);
        assert_eq!(classify_covenant(COV_OPEN), SpendClass::NameControl);
        assert_eq!(classify_covenant(COV_REGISTER), SpendClass::NameControl);
        assert_eq!(classify_covenant(COV_TRANSFER), SpendClass::NameControl);
        assert_eq!(classify_covenant(COV_REVOKE), SpendClass::Unsupported);
        assert_eq!(classify_covenant(99), SpendClass::Unsupported);
    }

    #[test]
    fn upsert_and_balances_split_by_class() {
        let conn = mem_db();
        upsert_utxo(&conn, "p1", &coin("aa", 0, 1_000_000, None)).unwrap();
        upsert_utxo(&conn, "p1", &coin("bb", 1, 2_000_000, Some(cov(COV_BID)))).unwrap();
        upsert_utxo(&conn, "p1", &coin("cc", 0, 3_000_000, Some(cov(COV_REGISTER)))).unwrap();

        let bal = compute_balances(&conn, "p1").unwrap();
        assert_eq!(bal.liquid, 1_000_000);
        assert_eq!(bal.name_lockup, 2_000_000);
        assert_eq!(bal.name_control, 3_000_000);
        assert_eq!(bal.total(), 6_000_000);
    }

    #[test]
    fn upsert_is_idempotent_and_refreshes() {
        let conn = mem_db();
        upsert_utxo(&conn, "p1", &coin("aa", 0, 1_000_000, None)).unwrap();
        // Re-upsert with a new value (e.g. reorg / confirmation update).
        let mut updated = coin("aa", 0, 1_500_000, None);
        updated.height = Some(105);
        upsert_utxo(&conn, "p1", &updated).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracked_utxos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let bal = compute_balances(&conn, "p1").unwrap();
        assert_eq!(bal.liquid, 1_500_000);
    }

    #[test]
    fn mark_missing_as_spent_excludes_from_balance() {
        let conn = mem_db();
        upsert_utxo(&conn, "p1", &coin("aa", 0, 1_000_000, None)).unwrap();
        upsert_utxo(&conn, "p1", &coin("bb", 0, 2_000_000, None)).unwrap();

        // Node now only reports "bb"; "aa" was spent.
        let live = vec![coin("bb", 0, 2_000_000, None)];
        let spent = mark_missing_as_spent(&conn, "p1", &live).unwrap();
        assert_eq!(spent, 1);

        let bal = compute_balances(&conn, "p1").unwrap();
        assert_eq!(bal.liquid, 2_000_000);

        // Re-running is a no-op (already marked).
        let spent_again = mark_missing_as_spent(&conn, "p1", &live).unwrap();
        assert_eq!(spent_again, 0);
    }

    #[test]
    fn reappearing_coin_is_unspent_again() {
        let conn = mem_db();
        upsert_utxo(&conn, "p1", &coin("aa", 0, 1_000_000, None)).unwrap();
        mark_missing_as_spent(&conn, "p1", &[]).unwrap();
        assert_eq!(compute_balances(&conn, "p1").unwrap().liquid, 0);
        // Coin re-appears (e.g. mempool double-spend reverted) — upsert clears spend.
        upsert_utxo(&conn, "p1", &coin("aa", 0, 1_000_000, None)).unwrap();
        assert_eq!(compute_balances(&conn, "p1").unwrap().liquid, 1_000_000);
    }

    #[test]
    fn sync_cursor_round_trips() {
        let conn = mem_db();
        assert_eq!(get_sync_height(&conn, "p1").unwrap(), 0);
        set_sync_cursor(&conn, "p1", 12345).unwrap();
        assert_eq!(get_sync_height(&conn, "p1").unwrap(), 12345);
        set_sync_cursor(&conn, "p1", 12400).unwrap();
        assert_eq!(get_sync_height(&conn, "p1").unwrap(), 12400);
    }

    #[test]
    fn upsert_name_state_handles_known_and_unknown() {
        let conn = mem_db();

        // Known name from getnameinfo.
        let info = serde_json::json!({
            "info": {
                "name": "example",
                "nameHash": "abcd1234",
                "state": "CLOSED",
                "height": 5000,
                "renewal": 5100,
                "renewals": 2,
                "weak": false,
                "owner": { "hash": "ffee", "index": 1 }
            }
        });
        upsert_name_state(&conn, "p1", "example", &info).unwrap();

        let (state, hash, owner_txid, owner_vout): (String, String, Option<String>, Option<i64>) =
            conn.query_row(
                "SELECT state, name_hash_hex, owner_txid, owner_vout
                 FROM tracked_name_states WHERE wallet_profile_id = 'p1' AND name = 'example'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(state, "CLOSED");
        assert_eq!(hash, "abcd1234");
        assert_eq!(owner_txid.as_deref(), Some("ffee"));
        assert_eq!(owner_vout, Some(1));

        // Unknown name (null info) gets a minimal UNKNOWN row.
        let unknown = serde_json::json!({ "info": null });
        upsert_name_state(&conn, "p1", "nope", &unknown).unwrap();
        let state: String = conn
            .query_row(
                "SELECT state FROM tracked_name_states
                 WHERE wallet_profile_id = 'p1' AND name = 'nope'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "UNKNOWN");

        // Re-upserting the same name updates in place (no duplicate row).
        let updated = serde_json::json!({
            "info": { "nameHash": "abcd1234", "state": "REVOKED" }
        });
        upsert_name_state(&conn, "p1", "example", &updated).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracked_name_states
                 WHERE wallet_profile_id = 'p1' AND name = 'example'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn cache_transaction_is_idempotent() {
        let conn = mem_db();
        cache_transaction(&conn, "p1", "tx1", Some(10), Some("2024-01-01"), "{}").unwrap();
        cache_transaction(&conn, "p1", "tx1", Some(11), Some("2024-01-02"), "{\"a\":1}").unwrap();
        let (count, height): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), MAX(height) FROM wallet_transactions_cache
                 WHERE wallet_profile_id = 'p1' AND txid = 'tx1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(height, 11);
    }
}
