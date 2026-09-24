//! Background chain scanner: indexes BID/REVEAL covenant outputs per name from
//! the fully-synced local hsd node's block data. Populates `name_bid_outpoints`
//! so `read_name_bids` can show ALL bidders (not just the wallet's own) without
//! touching the HNSFans explorer.
//!
//! Design:
//! - Runs only while the node is synced (`node_ready_from_settings`).
//! - Scoped to the active profile's network: both the cursor and the index are
//!   keyed by it, because a name hashes to the same value on every chain and a
//!   mainnet cursor height is meaningless against a regtest tip (see 028).
//! - Scoped to an auction: a name can be auctioned repeatedly, so rows also
//!   carry the OPEN height the covenant names (see 029).
//! - Walks blocks from this network's `chain_scan_cursor.last_height + 1` to
//!   the node tip.
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
/// Sleep duration when in SPV mode (chain scanner is disabled, no full blocks).
/// Longer than NOT_READY_SLEEP to reduce resource waste.
const SPV_SLEEP: Duration = Duration::from_secs(300);

/// Sleep when the scanner is caught up to the tip before polling for new blocks.
const CAUGHT_UP_SLEEP: Duration = Duration::from_secs(10);

/// Entry point: spawned as a background Tokio task from `lib.rs::setup()`.
/// Runs indefinitely, sleeping when the node isn't ready or the scanner is
/// caught up to the tip.
///
/// COVERAGE NOTE (deliberate, not an oversight): this function is an
/// untestable IO-shell driver loop — `loop { ... sleep().await; continue }`
/// that never returns. It only wires together IO (DB open, settings read,
/// node tip poll, RPC client construction) and delegates every unit of real
/// logic to helpers that ARE unit-tested: `scan_block` (covenant parsing +
/// DB upsert), `get_scan_cursor`/`set_scan_cursor` (cursor persistence) and
/// `read_indexed_bids` (read-back). We intentionally do not test the loop
/// body itself; there is no return path and no seam to observe, so exercising
/// it would require a live node and would still never terminate.
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn run_chain_scanner(db_path: String) {
    loop {
        // Settings and the active profile's network come from one connection:
        // the scanner must not treat a node on another chain as authoritative
        // (its heights and covenants would be written against our cursor).
        let (settings, expected_network, active_profile_id) = {
            let conn = match open_conn(&db_path) {
                Ok(c) => c,
                Err(_) => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            };
            let settings = match queries::get_settings(&conn) {
                Ok(s) => s,
                Err(_) => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            };
            let network = queries::get_active_profile_network(&conn).ok().flatten();
            let profile_id = queries::get_active_profile_id(&conn)
                .ok()
                .filter(|s| !s.is_empty());
            (settings, network, profile_id)
        };

        // Only scan when the node is authoritative.
        // In SPV mode, the node doesn't have full blocks, so the chain scanner
        // cannot walk transactions. Skip scanning and sleep longer (5 minutes)
        // to reduce resource waste.
        let node_mode = crate::noncustodial::rpc::resolve_node_mode(&settings);
        if node_mode.is_spv() {
            sleep(SPV_SLEEP).await;
            continue;
        }

        // The cursor and the index are network-keyed, so an unknown chain has
        // nowhere safe to write: attributing it to a default would file, say,
        // regtest BIDs under mainnet's nameHashes. No profile also means no
        // caller — `read_name_bids` is per-profile — so idling costs nothing.
        let scan_network = match expected_network
            .as_deref()
            .and_then(crate::noncustodial::network::Network::from_str_opt)
        {
            Some(n) => n.as_str(),
            None => {
                sleep(NOT_READY_SLEEP).await;
                continue;
            }
        };

        // Use per-profile probe if active profile exists; otherwise fall back to global.
        let tip = if let Some(profile_id) = active_profile_id.as_deref() {
            match crate::commands::node_readiness::node_tip_height_if_synced_from_profile_with_network(
                &db_path,
                profile_id,
                expected_network.as_deref(),
            )
            .await
            {
                Some(h) => h,
                None => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            }
        } else {
            match crate::commands::node_readiness::node_tip_height_if_synced_from_settings_with_network(
                &settings,
                expected_network.as_deref(),
            )
            .await
            {
                Some(h) => h,
                None => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
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
            get_scan_cursor(&conn, scan_network)
        };

        if cursor >= tip {
            // Caught up — wait for new blocks.
            sleep(CAUGHT_UP_SLEEP).await;
            continue;
        }

        // Resolve the client for the active profile if one exists, otherwise fall
        // back to global settings. Per-profile overrides take precedence per ADR-001.
        let client = if let Some(profile_id) = active_profile_id.as_deref() {
            let conn = match open_conn(&db_path) {
                Ok(c) => c,
                Err(_) => {
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            };
            match NodeRpcClient::for_profile(&conn, profile_id) {
                Ok(c) => c,
                Err(_) => {
                    // Profile not found or misconfigured — sleep and retry
                    sleep(NOT_READY_SLEEP).await;
                    continue;
                }
            }
        } else {
            // No active profile — fall back to global settings
            NodeRpcClient::from_settings(&settings)
        };
        let end = (cursor + BATCH_SIZE).min(tip);

        let mut advanced_to = cursor;
        for height in (cursor + 1)..=end {
            if scan_block(&client, &db_path, scan_network, height)
                .await
                .is_err()
            {
                // Transient RPC/DB error — stop this batch, retry next loop.
                break;
            }
            advanced_to = height;
        }

        // Advance cursor to the last successfully scanned height.
        if advanced_to > cursor {
            if let Ok(conn) = open_conn(&db_path) {
                let _ = set_scan_cursor(&conn, scan_network, advanced_to);
            }
        }

        sleep(BATCH_YIELD).await;
    }
}

/// Scan a single block: fetch via `getblock`, iterate outputs, upsert BID/REVEAL
/// covenants into `name_bid_outpoints`.
///
/// `pub(crate)` (not private) purely so the in-crate test module
/// `tests::chain_scan_tests` can drive it directly with a `MockNodeRpc` and a
/// file-backed temp DB. Zero behavior change — it is only ever called from
/// `run_chain_scanner` within this crate.
pub(crate) async fn scan_block(
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
    db_path: &str,
    network: &str,
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
        // `txid`, NOT `hash`: hsd's `txToJSON` puts the txid in `txid` and the
        // WITNESS txid in `hash` (`rpc.js` → `tx.txid()` / `tx.wtxid()`), and on
        // Handshake every signed tx has a witness, so the two always differ.
        // `bid_commitments.bid_txid` holds the txid, so indexing under the wtxid
        // meant a bid could never be recognised as the wallet's own.
        let txid = tx.get("txid").and_then(|h| h.as_str()).unwrap_or_default();
        if txid.is_empty() {
            continue;
        }
        // `vout`, NOT `outputs`: `getblock` goes through hsd's JSON-RPC layer,
        // which emits the bitcoind-shaped `vin`/`vout`. Only hsd's REST/HTTP API
        // uses `inputs`/`outputs` (what `noncustodial::sync` consumes). Reading
        // `outputs` here made every block parse as empty, so the scanner walked
        // the whole chain and indexed nothing while the cursor advanced to tip.
        let outputs = tx.get("vout").and_then(|o| o.as_array());
        let outputs = match outputs {
            Some(o) => o,
            None => continue,
        };
        // Same JSON-RPC shape as `vout`: `vin[i]` carries the spent outpoint as
        // `txid` + `vout` (hsd `txToJSON`).
        let inputs = tx.get("vin").and_then(|i| i.as_array());
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

            // item[1] of both BID and REVEAL is the u32 OPEN height of the
            // auction (`covenants::bid`/`covenants::reveal`'s `start`). It is
            // what separates this auction's bids from those of an earlier,
            // lapsed auction for the same name.
            let start_height = match items.get(1).and_then(|v| v.as_str()).and_then(u32le_hex) {
                Some(h) => h as i64,
                None => continue,
            };

            // `address` is an object here (`{version, hash, string}`), not the
            // bare bech32 string the REST API returns. Read the encoded form
            // hsd hands us; tolerate the flat shape for other node builds.
            let addr = output
                .get("address")
                .and_then(|a| {
                    a.get("string")
                        .and_then(|s| s.as_str())
                        .or_else(|| a.as_str())
                })
                .map(|s| s.to_string());
            // `value` is HNS as a JSON number (`Amount.coin(value, true)` →
            // `fixed.toFloat(value, 6)`), not doos. Every consumer of this table
            // — and `bid_commitments.lockup_value_doos` beside it — is in doos.
            let value = output
                .get("value")
                .and_then(|v| v.as_f64())
                .map(doos_from_hns)
                .unwrap_or(0);

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
                        name_start_height: start_height,
                        name: raw_name,
                        lockup_value_doos: value,
                        address: addr,
                        height,
                    });
                }
                COV_REVEAL => {
                    // REVEAL items: [nameHash, u32(height), nonce].
                    //
                    // Which bid it reveals is not a guess: hsd pairs a name
                    // covenant with the coin spent at the SAME index
                    // (`rules.verifyCovenants` walks `tx.inputs[i]` against
                    // `tx.output(i)`), so the BID being revealed is whatever
                    // input `vout` spends.
                    let spent = inputs.and_then(|v| v.get(vout)).and_then(|i| {
                        Some((
                            i.get("txid")?.as_str()?.to_string(),
                            i.get("vout")?.as_u64()? as u32,
                        ))
                    });
                    reveals.push(RevealRow {
                        name_hash_hex: name_hash,
                        name_start_height: start_height,
                        spent_bid: spent,
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
                (network, bid_txid, bid_vout, name_hash_hex, name,
                 name_start_height, lockup_value_doos, address, height)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(network, bid_txid, bid_vout) DO UPDATE SET
                name = COALESCE(excluded.name, name_bid_outpoints.name)",
            params![
                network,
                bid.bid_txid,
                bid.bid_vout,
                bid.name_hash_hex,
                bid.name,
                bid.name_start_height,
                bid.lockup_value_doos as i64,
                bid.address,
                bid.height,
            ],
        )?;
    }
    // Match REVEALs to their BIDs. A REVEAL output's value IS the true bid
    // value; the BID output's value is the lockup (bid + mask). The pairing is
    // exact, not a heuristic: each reveal names the outpoint of the bid it
    // spends, which is the only thing that still works once a wallet holds
    // several bids on one name.
    for reveal in &reveals {
        // Address the exact BID this reveal spends. The previous rule — "the
        // earliest bid not yet matched" — was indistinguishable from the truth
        // while a wallet held one bid per name, and scrambled the disclosed
        // values the moment one transaction revealed several.
        let Some((bid_txid, bid_vout)) = reveal.spent_bid.as_ref() else {
            continue;
        };
        tx.execute(
            "UPDATE name_bid_outpoints
             SET reveal_txid = ?1, reveal_value_doos = ?2
             WHERE network = ?3 AND bid_txid = ?4 AND bid_vout = ?5",
            params![
                reveal.reveal_txid,
                reveal.reveal_value_doos as i64,
                network,
                bid_txid,
                *bid_vout,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Decode a covenant item that holds a `pushU32` value: hex of up to 4
/// little-endian bytes. Shorter pushes are zero-extended; anything longer is
/// not a u32 push and yields `None`.
fn u32le_hex(hex_str: &str) -> Option<u32> {
    let bytes = hex::decode(hex_str).ok()?;
    if bytes.len() > 4 {
        return None;
    }
    let mut buf = [0u8; 4];
    buf[..bytes.len()].copy_from_slice(&bytes);
    Some(u32::from_le_bytes(buf))
}

/// Convert an HNS amount as hsd's JSON-RPC reports it (a float with at most 6
/// decimals) into doos. Exact for every reachable amount: the 2.04e9 HNS supply
/// cap is 2.04e15 doos, well inside f64's 2^53 exact-integer range.
fn doos_from_hns(hns: f64) -> u64 {
    if !hns.is_finite() || hns <= 0.0 {
        return 0;
    }
    (hns * 1_000_000.0).round() as u64
}

// --- DB helpers (chain_scan_cursor) ------------------------------------------

fn get_scan_cursor(conn: &rusqlite::Connection, network: &str) -> i64 {
    conn.query_row(
        "SELECT last_height FROM chain_scan_cursor WHERE network = ?1",
        params![network],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// `pub(crate)` so the in-crate test module can assert cursor persistence
/// end-to-end (write via `set_scan_cursor`, read via `scan_cursor_height`).
/// Zero behavior change — only ever called from `run_chain_scanner`.
pub(crate) fn set_scan_cursor(
    conn: &rusqlite::Connection,
    network: &str,
    height: i64,
) -> Result<(), crate::error::AppError> {
    conn.execute(
        "INSERT INTO chain_scan_cursor (network, last_height) VALUES (?1, ?2)
         ON CONFLICT(network) DO UPDATE SET last_height = excluded.last_height",
        params![network, height],
    )?; // COVERAGE: the Err branch of `?` requires a corrupt/closed DB — not unit-testable.
    Ok(())
}

// --- Internal row types ------------------------------------------------------

struct BidRow {
    bid_txid: String,
    bid_vout: u32,
    name_hash_hex: String,
    name_start_height: i64,
    name: Option<String>,
    lockup_value_doos: u64,
    address: Option<String>,
    height: i64,
}

struct RevealRow {
    name_hash_hex: String,
    name_start_height: i64,
    /// The BID outpoint this reveal spends — `(txid, vout)` of the input at
    /// the reveal output's own index. `None` only for a malformed tx.
    spent_bid: Option<(String, u32)>,
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
    network: &str,
    name_start_height: i64,
    name_hash_hex: &str,
) -> Result<Vec<crate::hsd::types::HsdBid>, crate::error::AppError> {
    let mut stmt = conn.prepare(
        "SELECT bid_txid, bid_vout, lockup_value_doos, reveal_value_doos, reveal_txid
         FROM name_bid_outpoints
         WHERE network = ?1 AND name_hash_hex = ?2 AND name_start_height = ?3
         ORDER BY height ASC, bid_txid ASC, bid_vout ASC",
    )?; // COVERAGE: the Err branch of `?` requires a malformed statement / broken DB — not unit-testable.
    let rows = stmt.query_map(
        params![
            network,
            name_hash_hex.to_ascii_lowercase(),
            name_start_height
        ],
        |r| {
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
        },
    )?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// The scanner's current cursor height. Exposed so `read_name_bids` can tell
/// whether the scanner has reached the name's auction window yet.
pub fn scan_cursor_height(conn: &rusqlite::Connection, network: &str) -> i64 {
    get_scan_cursor(conn, network)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both helpers are private to this module, so their tests live here rather
    // than in `tests/chain_scan_tests.rs`.

    #[test]
    fn u32le_hex_zero_extends_a_short_push() {
        // hsd's `pushU32` trims leading zero bytes, so a small height arrives
        // as fewer than four bytes and must not be read as a different number.
        assert_eq!(u32le_hex(""), Some(0));
        assert_eq!(u32le_hex("2a"), Some(42));
        assert_eq!(u32le_hex("2a00"), Some(42));
        assert_eq!(u32le_hex("2a000000"), Some(42));
        assert_eq!(u32le_hex("ffffffff"), Some(u32::MAX));
    }

    #[test]
    fn u32le_hex_refuses_what_is_not_a_u32_push() {
        // More than four bytes is some other covenant item — a name hash, say —
        // and truncating it would silently invent a height.
        assert_eq!(u32le_hex("2a0000000000"), None);
        // Not hex at all.
        assert_eq!(u32le_hex("zz"), None);
        // An odd number of hex digits is not a byte string.
        assert_eq!(u32le_hex("abc"), None);
    }

    #[test]
    fn doos_from_hns_converts_the_amounts_hsd_reports() {
        assert_eq!(doos_from_hns(1.0), 1_000_000);
        assert_eq!(doos_from_hns(0.000001), 1);
        // Six decimals is hsd's full precision; rounding is exact there.
        assert_eq!(doos_from_hns(12.345678), 12_345_678);
        // Well inside f64's exact-integer range, as the doc claims: the whole
        // supply cap converts without loss.
        assert_eq!(doos_from_hns(2_040_000_000.0), 2_040_000_000_000_000);
    }

    #[test]
    fn doos_from_hns_treats_nothing_and_nonsense_as_zero() {
        assert_eq!(doos_from_hns(0.0), 0);
        assert_eq!(doos_from_hns(-1.0), 0);
        assert_eq!(doos_from_hns(f64::NAN), 0);
        assert_eq!(doos_from_hns(f64::INFINITY), 0);
    }
}
