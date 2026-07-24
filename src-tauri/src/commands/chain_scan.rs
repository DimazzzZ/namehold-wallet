//! Background chain scanner: indexes BID/REVEAL covenant outputs per name from
//! the fully-synced local hsd node's block data. Populates `name_bid_outpoints`
//! so `read_name_bids` can show ALL bidders (not just the wallet's own) without
//! touching the HNSFans explorer.
//!
//! Design:
//! - Runs only while the node is synced (`node_ready_from_settings`).
//! - Walks blocks from `chain_scan_cursor.last_height + 1` to the node tip.
//! - For each block: `getblock(hash, verbose, verboseTx)` → iterate outputs →
//!   BID/REVEAL covenants → upsert into `name_bid_outpoints`.
//! - Advances the cursor per block so it's resumable and never re-scans genesis.
//! - Throttled: yields between blocks so it never starves UI-facing RPC.
//! - Spawned from `lib.rs::setup()` as a background Tokio task.

use crate::commands::sync::open_conn;
use crate::db::queries;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::noncustodial::sync::{COV_BID, COV_REVEAL};
use rusqlite::params;
use std::time::Duration;
use tokio::time::sleep;

/// How many blocks to process in one batch before yielding. Keeps the scanner
/// from monopolizing the node RPC and the DB write lock.
const BATCH_SIZE: i64 = 50;

/// Sleep between batches to let other RPC callers through.
const BATCH_YIELD: Duration = Duration::from_millis(100);

/// Sleep when the node is not ready (disconnected or syncing) before rechecking.
const NOT_READY_SLEEP: Duration = Duration::from_secs(30);

/// Sleep when the scanner is caught up to the tip before polling for new blocks.
const CAUGHT_UP_SLEEP: Duration = Duration::from_secs(10);

/// Entry point: spawned as a background Tokio task from `lib.rs::setup()`.
/// Runs indefinitely, sleeping when the node isn't ready or the scanner is
/// caught up to the tip.
pub async fn run_chain_scanner(db_path: String) {
    loop {
        let settings = {
            let conn = match open_conn(&db_path) {
                Ok(c) => c,
                Err(_) => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            };
            match queries::get_settings(&conn) {
                Ok(s) => s,
                Err(_) => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            }
        };

        // Only scan when the node is authoritative.
        let tip =
            match crate::commands::read::node_tip_height_if_synced_from_settings(&settings).await {
                Some(h) => h,
                None => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            };

        let cursor = {
            let conn = match open_conn(&db_path) {
                Ok(c) => c,
                Err(_) => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            };
            get_scan_cursor(&conn)
        };

        if cursor >= tip {
            // Caught up — wait for new blocks.
            sleep(CAUGHT_UP_SLEEP).await;
            continue;
        }

        let client = NodeRpcClient::from_settings(&settings);
        let end = (cursor + BATCH_SIZE).min(tip);

        let mut advanced_to = cursor;
        for height in (cursor + 1)..=end {
            if scan_block(&client, &db_path, height).await.is_err() {
                // Transient RPC/DB error — stop this batch, retry next loop.
                break;
            }
            advanced_to = height;
        }

        // Advance cursor to the last successfully scanned height.
        if advanced_to > cursor {
            if let Ok(conn) = open_conn(&db_path) {
                let _ = set_scan_cursor(&conn, advanced_to);
            }
        }

        sleep(BATCH_YIELD).await;
    }
}

/// Scan a single block: fetch via `getblock`, iterate outputs, upsert BID/REVEAL
/// covenants into `name_bid_outpoints`.
async fn scan_block(
    client: &NodeRpcClient,
    db_path: &str,
    height: i64,
) -> Result<(), crate::error::AppError> {
    let hash = client.get_block_hash(height).await?;
    let block = client.get_block(&hash).await?;

    let txs = block.get("tx").and_then(|t| t.as_array());
    let txs = match txs {
        Some(t) => t,
        None => return Ok(()), // empty or malformed block — skip
    };

    let mut bids: Vec<BidRow> = Vec::new();
    let mut reveals: Vec<RevealRow> = Vec::new();

    for tx in txs {
        let txid = tx.get("hash").and_then(|h| h.as_str()).unwrap_or_default();
        if txid.is_empty() {
            continue;
        }
        let outputs = tx.get("outputs").and_then(|o| o.as_array());
        let outputs = match outputs {
            Some(o) => o,
            None => continue,
        };
        for (vout, output) in outputs.iter().enumerate() {
            let cov = match output.get("covenant") {
                Some(c) => c,
                None => continue,
            };
            let cov_type = cov.get("type").and_then(|t| t.as_u64()).unwrap_or(0) as u8;
            let items = cov.get("items").and_then(|i| i.as_array());
            let items = match items {
                Some(i) if !i.is_empty() => i,
                _ => continue,
            };

            let name_hash = items[0].as_str().unwrap_or_default().to_ascii_lowercase();
            if name_hash.is_empty() {
                continue;
            }

            let addr = output
                .get("address")
                .and_then(|a| a.as_str())
                .map(|s| s.to_string());
            let value = output.get("value").and_then(|v| v.as_u64()).unwrap_or(0);

            match cov_type {
                COV_BID => {
                    // BID items: [nameHash, u32(start), rawName, blind]
                    let raw_name = items
                        .get(2)
                        .and_then(|r| r.as_str())
                        .and_then(|h| hex::decode(h).ok())
                        .and_then(|b| String::from_utf8(b).ok());
                    bids.push(BidRow {
                        bid_txid: txid.to_string(),
                        bid_vout: vout as u32,
                        name_hash_hex: name_hash,
                        name: raw_name,
                        lockup_value_doos: value,
                        address: addr,
                        height,
                    });
                }
                COV_REVEAL => {
                    // REVEAL items: [nameHash, u32(height), nonce]
                    reveals.push(RevealRow {
                        name_hash_hex: name_hash,
                        reveal_txid: txid.to_string(),
                        reveal_value_doos: value,
                    });
                }
                _ => {}
            }
        }
    }

    if bids.is_empty() && reveals.is_empty() {
        return Ok(());
    }

    let conn = open_conn(db_path)?;
    let tx = conn.unchecked_transaction()?;
    for bid in &bids {
        tx.execute(
            "INSERT INTO name_bid_outpoints
                (bid_txid, bid_vout, name_hash_hex, name, lockup_value_doos, address, height)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(bid_txid, bid_vout) DO UPDATE SET
                name = COALESCE(excluded.name, name_bid_outpoints.name)",
            params![
                bid.bid_txid,
                bid.bid_vout,
                bid.name_hash_hex,
                bid.name,
                bid.lockup_value_doos as i64,
                bid.address,
                bid.height,
            ],
        )?;
    }
    // Match REVEALs to their BIDs by nameHash (within the same block or earlier
    // blocks). A REVEAL output's value IS the true bid value; the BID output's
    // value is the lockup (bid + mask). We update the FIRST matching BID that
    // doesn't already have a reveal_txid — this is a best-effort heuristic;
    // in practice each bidder has one BID per name per auction.
    for reveal in &reveals {
        tx.execute(
            "UPDATE name_bid_outpoints
             SET reveal_txid = ?1, reveal_value_doos = ?2
             WHERE rowid = (
                 SELECT rowid FROM name_bid_outpoints
                 WHERE name_hash_hex = ?3 AND reveal_txid IS NULL
                 ORDER BY height ASC, bid_txid ASC, bid_vout ASC
                 LIMIT 1
             )",
            params![
                reveal.reveal_txid,
                reveal.reveal_value_doos as i64,
                reveal.name_hash_hex,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

// --- DB helpers (chain_scan_cursor) ------------------------------------------

fn get_scan_cursor(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
        "SELECT last_height FROM chain_scan_cursor WHERE id = 1",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

fn set_scan_cursor(conn: &rusqlite::Connection, height: i64) -> Result<(), crate::error::AppError> {
    conn.execute(
        "UPDATE chain_scan_cursor SET last_height = ?1 WHERE id = 1",
        params![height],
    )?;
    Ok(())
}

// --- Internal row types ------------------------------------------------------

struct BidRow {
    bid_txid: String,
    bid_vout: u32,
    name_hash_hex: String,
    name: Option<String>,
    lockup_value_doos: u64,
    address: Option<String>,
    height: i64,
}

struct RevealRow {
    name_hash_hex: String,
    reveal_txid: String,
    reveal_value_doos: u64,
}

// --- Query for read_name_bids ------------------------------------------------

/// Read all indexed bids for a name (by nameHash) from the chain scanner's
/// `name_bid_outpoints` table. Returns them shaped as `HsdBid` values so the
/// caller (`read_name_bids`) can merge with the wallet's own `bid_commitments`
/// and return the same JSON the frontend expects.
pub fn read_indexed_bids(
    conn: &rusqlite::Connection,
    name_hash_hex: &str,
) -> Result<Vec<crate::hsd::types::HsdBid>, crate::error::AppError> {
    let mut stmt = conn.prepare(
        "SELECT bid_txid, bid_vout, lockup_value_doos, reveal_value_doos, reveal_txid
         FROM name_bid_outpoints
         WHERE name_hash_hex = ?1
         ORDER BY height ASC, bid_txid ASC, bid_vout ASC",
    )?;
    let rows = stmt.query_map(params![name_hash_hex.to_ascii_lowercase()], |r| {
        let txid: String = r.get(0)?;
        let index: u32 = r.get::<_, i64>(1)? as u32;
        let lockup: i64 = r.get(2)?;
        let reveal_value: Option<i64> = r.get(3)?;
        let reveal_txid: Option<String> = r.get(4)?;
        Ok(crate::hsd::types::HsdBid {
            txid: Some(txid),
            index: Some(index),
            lockup: Some(lockup as u64),
            value: reveal_value.map(|v| v as u64),
            revealed: Some(reveal_txid.is_some()),
            win: None,
            reveal: None,
            time: None,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// The scanner's current cursor height. Exposed so `read_name_bids` can tell
/// whether the scanner has reached the name's auction window yet.
pub fn scan_cursor_height(conn: &rusqlite::Connection) -> i64 {
    get_scan_cursor(conn)
}
