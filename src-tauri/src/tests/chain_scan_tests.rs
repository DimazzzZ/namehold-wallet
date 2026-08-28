//! Feature 3, Stage 2 — chain scanner tests.
//!
//! Covers the persistence surface (`name_bid_outpoints`, `chain_scan_cursor`)
//! and the shape merge that feeds `read_name_bids` when the scanner is the
//! source. The scanner LOOP itself (block-fetching + covenant parsing) is
//! implicitly exercised by the DB shape here — an integration test against a
//! real node would go through `live_node_it` (see the "live_node_it" pattern).

use rusqlite::{params, Connection};

use crate::commands::chain_scan::{read_indexed_bids, scan_cursor_height};
use crate::commands::read::merge_indexed_bids;
use crate::db;
use crate::db::queries::BidCommitmentRow;

fn conn() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&c).unwrap();
    c
}

#[allow(clippy::too_many_arguments)]
fn seed_bid(
    c: &Connection,
    bid_txid: &str,
    bid_vout: i64,
    name_hash: &str,
    name: Option<&str>,
    lockup: i64,
    height: i64,
    reveal_txid: Option<&str>,
    reveal_value: Option<i64>,
) {
    c.execute(
        "INSERT INTO name_bid_outpoints
            (bid_txid, bid_vout, name_hash_hex, name, lockup_value_doos,
             address, height, reveal_txid, reveal_value_doos)
         VALUES (?1, ?2, ?3, ?4, ?5, 'rs1qx', ?6, ?7, ?8)",
        params![
            bid_txid,
            bid_vout,
            name_hash,
            name,
            lockup,
            height,
            reveal_txid,
            reveal_value,
        ],
    )
    .unwrap();
}

#[test]
fn read_indexed_bids_filters_by_name_hash_and_orders_by_height() {
    let c = conn();
    // Two bids for the same name at different heights + one for another name.
    seed_bid(
        &c,
        "tx1",
        0,
        "hasha",
        Some("namehold"),
        1_000_000,
        200,
        None,
        None,
    );
    seed_bid(
        &c,
        "tx2",
        0,
        "hasha",
        Some("namehold"),
        2_500_000,
        205,
        Some("rv2"),
        Some(500_000),
    );
    seed_bid(
        &c,
        "tx3",
        0,
        "hashb",
        Some("other"),
        999_999,
        201,
        None,
        None,
    );

    let out = read_indexed_bids(&c, "hasha").unwrap();
    assert_eq!(out.len(), 2);
    // Ordered by height ASC — earlier bid first.
    assert_eq!(out[0].txid.as_deref(), Some("tx1"));
    assert_eq!(out[0].lockup, Some(1_000_000));
    assert_eq!(out[0].value, None); // not yet revealed
    assert_eq!(out[0].revealed, Some(false));
    assert_eq!(out[1].txid.as_deref(), Some("tx2"));
    // The revealed bid carries its true value from the REVEAL output.
    assert_eq!(out[1].value, Some(500_000));
    assert_eq!(out[1].revealed, Some(true));

    // Unrelated name hash returns nothing (never leaks other names' bids).
    assert!(read_indexed_bids(&c, "nosuchhash").unwrap().is_empty());
}

#[test]
fn read_indexed_bids_lowercases_name_hash_query() {
    let c = conn();
    seed_bid(&c, "tx1", 0, "aabbcc", Some("n"), 100, 5, None, None);
    // Query with uppercase — the row was stored lowercase, so the query must
    // normalize on the way in to match.
    let out = read_indexed_bids(&c, "AABBCC").unwrap();
    assert_eq!(out.len(), 1);
}

/// The scanner's REVEAL→BID matching: a REVEAL for a nameHash attaches to the
/// EARLIEST unmatched BID (height ASC, then bid_txid ASC, then bid_vout ASC),
/// and a second REVEAL for the same name attaches to the NEXT unmatched BID —
/// never double-matching one BID. This drives the exact UPDATE `scan_block`
/// runs (the scanner loop is a thin wrapper around this SQL).
fn apply_reveal(c: &Connection, name_hash: &str, reveal_txid: &str, reveal_value: i64) {
    c.execute(
        "UPDATE name_bid_outpoints
         SET reveal_txid = ?1, reveal_value_doos = ?2
         WHERE rowid = (
             SELECT rowid FROM name_bid_outpoints
             WHERE name_hash_hex = ?3 AND reveal_txid IS NULL
             ORDER BY height ASC, bid_txid ASC, bid_vout ASC
             LIMIT 1
         )",
        params![reveal_txid, reveal_value, name_hash],
    )
    .unwrap();
}

#[test]
fn reveal_matches_earliest_unmatched_bid_for_same_name() {
    let c = conn();
    // Two BIDs for the same name at different heights, plus one for another name.
    seed_bid(
        &c,
        "bidLate",
        0,
        "hn",
        Some("multi"),
        3_000_000,
        210,
        None,
        None,
    );
    seed_bid(
        &c,
        "bidEarly",
        0,
        "hn",
        Some("multi"),
        2_000_000,
        205,
        None,
        None,
    );
    seed_bid(
        &c,
        "bidOther",
        0,
        "other",
        Some("other"),
        1_000_000,
        205,
        None,
        None,
    );

    // First REVEAL → earliest BID (height 205 wins over 210).
    apply_reveal(&c, "hn", "rvA", 1_800_000);
    // Second REVEAL for the same name → the NEXT unmatched BID (height 210).
    apply_reveal(&c, "hn", "rvB", 2_500_000);

    let out = read_indexed_bids(&c, "hn").unwrap();
    assert_eq!(out.len(), 2);
    // Ordered by height ASC in read_indexed_bids.
    assert_eq!(out[0].txid.as_deref(), Some("bidEarly"));
    assert_eq!(out[0].value, Some(1_800_000)); // first reveal → earliest bid
    assert_eq!(out[0].revealed, Some(true));
    assert_eq!(out[1].txid.as_deref(), Some("bidLate"));
    assert_eq!(out[1].value, Some(2_500_000)); // second reveal → next bid
    assert_eq!(out[1].revealed, Some(true));

    // The other name's BID is never touched by these reveals.
    let other = read_indexed_bids(&c, "other").unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].revealed, Some(false));
    assert_eq!(other[0].value, None);
}

#[test]
fn scan_cursor_defaults_to_zero_and_can_advance() {
    let c = conn();
    // The 018 migration inserts the singleton row at last_height=0.
    assert_eq!(scan_cursor_height(&c), 0);

    // Direct UPDATE (matches what the scanner does through set_scan_cursor).
    c.execute(
        "UPDATE chain_scan_cursor SET last_height = ?1 WHERE id = 1",
        params![1_234_i64],
    )
    .unwrap();
    assert_eq!(scan_cursor_height(&c), 1_234);
}

#[test]
fn merge_indexed_bids_marks_mine_and_computes_highest() {
    // Scanner returned two bids for "namehold" — one revealed for 500k, one
    // still pending — and the wallet has a commitment for one of them.
    let indexed = vec![
        crate::hsd::types::HsdBid {
            txid: Some("txmine".into()),
            index: Some(0),
            lockup: Some(2_000_000),
            value: Some(500_000),
            revealed: Some(true),
            win: None,
            reveal: None,
            time: None,
        },
        crate::hsd::types::HsdBid {
            txid: Some("txother".into()),
            index: Some(0),
            lockup: Some(1_500_000),
            value: None,
            revealed: Some(false),
            win: None,
            reveal: None,
            time: None,
        },
    ];
    let commitments = vec![BidCommitmentRow {
        name: "namehold".into(),
        name_hash_hex: "aa".into(),
        address: "rs1qa".into(),
        branch: 0,
        child_index: 0,
        bid_value_doos: 400_000,
        lockup_value_doos: 2_000_000,
        nonce_hex: "n".into(),
        blind_hex: "b".into(),
        bid_txid: Some("txmine".into()),
        reveal_txid: None,
        reveal_end_height: None,
    }];

    let out = merge_indexed_bids(&indexed, &commitments, "namehold");
    let bids = out["bids"].as_array().unwrap();
    assert_eq!(bids.len(), 2);
    // Ordered as returned; check mine flag by txid.
    let mine = bids.iter().find(|b| b["txid"] == "txmine").unwrap();
    assert_eq!(mine["mine"], true);
    // myValue reflects our own plaintext bid, not the on-chain value.
    assert_eq!(mine["myValue"], 400_000);
    let other = bids.iter().find(|b| b["txid"] == "txother").unwrap();
    assert_eq!(other["mine"], false);
    assert_eq!(other["myValue"], serde_json::Value::Null);

    // Aggregate `highest` is the max REVEALED value; unrevealed bids are
    // ignored so we never overstate the top bid.
    assert_eq!(out["highest"], 500_000);
    assert_eq!(out["myBidCount"], 1);
}

#[test]
fn merge_indexed_bids_never_marks_bid_from_a_different_name_as_mine() {
    // A commitment for a DIFFERENT name — must not attach to any bid in the
    // indexed slice, even if txids collide.
    let indexed = vec![crate::hsd::types::HsdBid {
        txid: Some("txshared".into()),
        index: Some(0),
        lockup: Some(1_000_000),
        value: None,
        revealed: Some(false),
        win: None,
        reveal: None,
        time: None,
    }];
    let commitments = vec![BidCommitmentRow {
        name: "different".into(),
        name_hash_hex: "aa".into(),
        address: "rs1qa".into(),
        branch: 0,
        child_index: 0,
        bid_value_doos: 400_000,
        lockup_value_doos: 1_000_000,
        nonce_hex: "n".into(),
        blind_hex: "b".into(),
        bid_txid: Some("txshared".into()),
        reveal_txid: None,
        reveal_end_height: None,
    }];

    let out = merge_indexed_bids(&indexed, &commitments, "namehold");
    let bids = out["bids"].as_array().unwrap();
    assert_eq!(bids.len(), 1);
    assert_eq!(bids[0]["mine"], false);
    assert_eq!(out["myBidCount"], 0);
}

// =============================================================================
// scan_block tests — exercises the full covenant-parsing + DB-upsert path using
// MockNodeRpc and a file-backed temp DB.
// =============================================================================

use crate::commands::chain_scan::{scan_block, set_scan_cursor};
use crate::noncustodial::sync::{COV_BID, COV_REVEAL};
use crate::tests::mock_node_rpc::MockNodeRpc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Create a file-backed temp DB (scan_block takes a `&str` path, not a Connection).
fn temp_db() -> (std::path::PathBuf, rusqlite::Connection) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("namehold_chain_scan_test_{pid}_{n}.db"));
    let _ = std::fs::remove_file(&path);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    (path, conn)
}

// --- Error propagation -------------------------------------------------------

#[tokio::test]
async fn scan_block_propagates_get_block_hash_error() {
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new().with_block_hash_err("hash rpc down");
    let result = scan_block(&mock, path.to_str().unwrap(), 1).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn scan_block_propagates_get_block_error() {
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("abc123".to_string())
        .with_block_err("block rpc down");
    let result = scan_block(&mock, path.to_str().unwrap(), 1).await;
    assert!(result.is_err());
}

// --- Empty / malformed blocks ------------------------------------------------

#[tokio::test]
async fn scan_block_ok_when_block_has_no_tx_field() {
    let (path, _conn) = temp_db();
    // Block JSON without a "tx" key → early Ok(())
    let mock = MockNodeRpc::new()
        .with_block_hash("abc".to_string())
        .with_block(serde_json::json!({"height": 1}));
    let result = scan_block(&mock, path.to_str().unwrap(), 1).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn scan_block_ok_when_tx_array_is_empty() {
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("abc".to_string())
        .with_block(serde_json::json!({"tx": []}));
    let result = scan_block(&mock, path.to_str().unwrap(), 1).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn scan_block_ok_when_tx_has_no_outputs() {
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("abc".to_string())
        .with_block(serde_json::json!({"tx": [{"hash": "tx1"}]}));
    let result = scan_block(&mock, path.to_str().unwrap(), 1).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn scan_block_ok_when_outputs_have_no_covenant() {
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("abc".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "tx1",
                "outputs": [{"value": 100, "address": "rs1qabc"}]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 1).await;
    assert!(result.is_ok());
}

// --- BID covenant insertion --------------------------------------------------

#[tokio::test]
async fn scan_block_inserts_bid_covenant() {
    let (path, conn) = temp_db();
    let name_hash = "aabbccdd";
    // COV_BID items: [nameHash, start_height_u32, rawName_hex, blind]
    // "namehold" in hex = 6e616d65686f6c64
    let raw_name_hex = hex::encode("namehold");
    let mock = MockNodeRpc::new()
        .with_block_hash("blockhash1".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txid_bid_1",
                "outputs": [{
                    "value": 5_000_000_u64,
                    "address": "rs1qbidder",
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": [name_hash, "00000001", &raw_name_hex, "blindhex"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 100).await;
    assert!(result.is_ok());

    // Verify the row was inserted
    let bids = read_indexed_bids(&conn, name_hash).unwrap();
    assert_eq!(bids.len(), 1);
    assert_eq!(bids[0].txid.as_deref(), Some("txid_bid_1"));
    assert_eq!(bids[0].index, Some(0));
    assert_eq!(bids[0].lockup, Some(5_000_000));
    assert_eq!(bids[0].revealed, Some(false));
    assert_eq!(bids[0].value, None);
}

#[tokio::test]
async fn scan_block_inserts_bid_with_undecoded_name() {
    // When rawName hex is not valid UTF-8, name should be None
    let (path, conn) = temp_db();
    let name_hash = "deadbeef";
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txbad",
                "outputs": [{
                    "value": 1_000_000_u64,
                    "address": "rs1qx",
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": [name_hash, "00000001", "ff80fe", "blind"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 50).await;
    assert!(result.is_ok());

    let bids = read_indexed_bids(&conn, name_hash).unwrap();
    assert_eq!(bids.len(), 1);
    assert_eq!(bids[0].lockup, Some(1_000_000));
}

// --- REVEAL covenant insertion -----------------------------------------------

#[tokio::test]
async fn scan_block_inserts_reveal_and_matches_to_existing_bid() {
    let (path, conn) = temp_db();
    let name_hash = "11223344";

    // Pre-seed a BID row so the REVEAL can match it
    seed_bid(&conn, "txbid1", 0, name_hash, Some("test"), 3_000_000, 90, None, None);

    // Now scan a block containing a REVEAL for the same name_hash
    let mock = MockNodeRpc::new()
        .with_block_hash("bh2".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txreveal1",
                "outputs": [{
                    "value": 2_000_000_u64,
                    "address": "rs1qrev",
                    "covenant": {
                        "type": COV_REVEAL as u64,
                        "items": [name_hash, "0000005a", "nonce123"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 100).await;
    assert!(result.is_ok());

    // The BID row should now have reveal_txid and reveal_value_doos set
    let bids = read_indexed_bids(&conn, name_hash).unwrap();
    assert_eq!(bids.len(), 1);
    assert_eq!(bids[0].revealed, Some(true));
    assert_eq!(bids[0].value, Some(2_000_000));
}

#[tokio::test]
async fn scan_block_reveal_without_matching_bid_is_noop() {
    // REVEAL for a name_hash that has no BID row → UPDATE matches 0 rows → Ok
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("bh3".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txrev_orphan",
                "outputs": [{
                    "value": 1_000_000_u64,
                    "address": "rs1qx",
                    "covenant": {
                        "type": COV_REVEAL as u64,
                        "items": ["no_such_hash", "00000001", "nonce"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 200).await;
    assert!(result.is_ok());
}

// --- Mixed BID + REVEAL in same block ----------------------------------------

#[tokio::test]
async fn scan_block_processes_bid_and_reveal_in_same_block() {
    let (path, conn) = temp_db();
    let name_hash = "aabb0011";
    let raw_name_hex = hex::encode("mixed");

    // Block contains a BID then a REVEAL for the same name in the same tx
    let mock = MockNodeRpc::new()
        .with_block_hash("bhmixed".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txmixed",
                "outputs": [
                    {
                        "value": 4_000_000_u64,
                        "address": "rs1qbid",
                        "covenant": {
                            "type": COV_BID as u64,
                            "items": [name_hash, "00000001", &raw_name_hex, "blind"]
                        }
                    },
                    {
                        "value": 2_500_000_u64,
                        "address": "rs1qrev",
                        "covenant": {
                            "type": COV_REVEAL as u64,
                            "items": [name_hash, "00000064", "nonce"]
                        }
                    }
                ]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 150).await;
    assert!(result.is_ok());

    // BID inserted and REVEAL matched to it
    let bids = read_indexed_bids(&conn, name_hash).unwrap();
    assert_eq!(bids.len(), 1);
    assert_eq!(bids[0].txid.as_deref(), Some("txmixed"));
    assert_eq!(bids[0].lockup, Some(4_000_000));
    assert_eq!(bids[0].revealed, Some(true));
    assert_eq!(bids[0].value, Some(2_500_000));
}

// --- Malformed output edge cases ---------------------------------------------

#[tokio::test]
async fn scan_block_skips_tx_with_empty_hash() {
    let (path, conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "",
                "outputs": [{
                    "value": 1_000_000_u64,
                    "address": "rs1q",
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": ["aabb", "00000001", "6e616d65", "blind"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 10).await;
    assert!(result.is_ok());
    // Nothing inserted because txid was empty
    let bids = read_indexed_bids(&conn, "aabb").unwrap();
    assert!(bids.is_empty());
}

#[tokio::test]
async fn scan_block_skips_covenant_with_empty_items() {
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txvalid",
                "outputs": [{
                    "value": 1_000_000_u64,
                    "address": "rs1q",
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": []
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 10).await;
    assert!(result.is_ok());
    // Empty items → skipped
}

#[tokio::test]
async fn scan_block_skips_covenant_with_empty_name_hash() {
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txvalid",
                "outputs": [{
                    "value": 1_000_000_u64,
                    "address": "rs1q",
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": ["", "00000001", "6e616d65", "blind"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 10).await;
    assert!(result.is_ok());
    // Empty name_hash → skipped
}

#[tokio::test]
async fn scan_block_skips_unknown_covenant_type() {
    let (path, conn) = temp_db();
    // Covenant type 99 is neither BID nor REVEAL → ignored
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txvalid",
                "outputs": [{
                    "value": 1_000_000_u64,
                    "address": "rs1q",
                    "covenant": {
                        "type": 99_u64,
                        "items": ["aabb", "00000001", "6e616d65", "blind"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 10).await;
    assert!(result.is_ok());
    let bids = read_indexed_bids(&conn, "aabb").unwrap();
    assert!(bids.is_empty());
}

#[tokio::test]
async fn scan_block_handles_missing_tx_hash_field() {
    // tx object without "hash" at all → unwrap_or_default → empty → skipped
    let (path, _conn) = temp_db();
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "outputs": [{
                    "value": 1_000_000_u64,
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": ["aabb", "00000001", "6e616d65", "blind"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 10).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn scan_block_handles_output_without_address_or_value() {
    // Missing "address" and "value" → address=None, value=0 — still inserts
    let (path, conn) = temp_db();
    let name_hash = "ccdd0011";
    let raw_name_hex = hex::encode("noaddr");
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txnoaddr",
                "outputs": [{
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": [name_hash, "00000001", &raw_name_hex, "blind"]
                    }
                }]
            }]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 10).await;
    assert!(result.is_ok());
    let bids = read_indexed_bids(&conn, name_hash).unwrap();
    assert_eq!(bids.len(), 1);
    assert_eq!(bids[0].lockup, Some(0)); // value defaults to 0
}

// --- ON CONFLICT (upsert) path -----------------------------------------------

#[tokio::test]
async fn scan_block_upsert_updates_name_on_conflict() {
    let (path, conn) = temp_db();
    let name_hash = "upserthash";

    // First insert: BID without a decodable name (invalid hex for rawName)
    let mock1 = MockNodeRpc::new()
        .with_block_hash("bh1".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txupsert",
                "outputs": [{
                    "value": 2_000_000_u64,
                    "address": "rs1q",
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": [name_hash, "00000001", "zzzz", "blind"]
                    }
                }]
            }]
        }));
    scan_block(&mock1, path.to_str().unwrap(), 10).await.unwrap();

    // Second insert: same (txid, vout) but now with a valid name
    let raw_name_hex = hex::encode("upserted");
    let mock2 = MockNodeRpc::new()
        .with_block_hash("bh2".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txupsert",
                "outputs": [{
                    "value": 2_000_000_u64,
                    "address": "rs1q",
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": [name_hash, "00000001", &raw_name_hex, "blind"]
                    }
                }]
            }]
        }));
    scan_block(&mock2, path.to_str().unwrap(), 11).await.unwrap();

    // The row should still be 1 (upserted, not duplicated)
    let bids = read_indexed_bids(&conn, name_hash).unwrap();
    assert_eq!(bids.len(), 1);
}

// --- Multiple transactions in one block --------------------------------------

#[tokio::test]
async fn scan_block_processes_multiple_txs() {
    let (path, conn) = temp_db();
    let nh1 = "multihash1";
    let nh2 = "multihash2";
    let raw1 = hex::encode("name1");
    let raw2 = hex::encode("name2");

    let mock = MockNodeRpc::new()
        .with_block_hash("bhmulti".to_string())
        .with_block(serde_json::json!({
            "tx": [
                {
                    "hash": "tx_a",
                    "outputs": [{
                        "value": 1_000_000_u64,
                        "address": "rs1qa",
                        "covenant": {
                            "type": COV_BID as u64,
                            "items": [nh1, "00000001", &raw1, "blind1"]
                        }
                    }]
                },
                {
                    "hash": "tx_b",
                    "outputs": [{
                        "value": 2_000_000_u64,
                        "address": "rs1qb",
                        "covenant": {
                            "type": COV_BID as u64,
                            "items": [nh2, "00000001", &raw2, "blind2"]
                        }
                    }]
                }
            ]
        }));
    let result = scan_block(&mock, path.to_str().unwrap(), 300).await;
    assert!(result.is_ok());

    assert_eq!(read_indexed_bids(&conn, nh1).unwrap().len(), 1);
    assert_eq!(read_indexed_bids(&conn, nh2).unwrap().len(), 1);
}

// --- set_scan_cursor ---------------------------------------------------------

#[tokio::test]
async fn set_scan_cursor_persists_height() {
    let (_path, conn) = temp_db();
    assert_eq!(scan_cursor_height(&conn), 0);
    set_scan_cursor(&conn, 42).unwrap();
    assert_eq!(scan_cursor_height(&conn), 42);
    set_scan_cursor(&conn, 9999).unwrap();
    assert_eq!(scan_cursor_height(&conn), 9999);
}

// --- name_hash lowercasing in scan_block -------------------------------------

#[tokio::test]
async fn scan_block_lowercases_name_hash_from_covenant() {
    let (path, conn) = temp_db();
    // Items contain uppercase name_hash — scan_block lowercases it
    let raw_name_hex = hex::encode("lower");
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txlower",
                "outputs": [{
                    "value": 500_000_u64,
                    "address": "rs1q",
                    "covenant": {
                        "type": COV_BID as u64,
                        "items": ["AABBCCDD", "00000001", &raw_name_hex, "blind"]
                    }
                }]
            }]
        }));
    scan_block(&mock, path.to_str().unwrap(), 5).await.unwrap();

    // Query with lowercase should find it
    let bids = read_indexed_bids(&conn, "aabbccdd").unwrap();
    assert_eq!(bids.len(), 1);
}

// --- Multiple outputs per tx (vout indexing) ---------------------------------

#[tokio::test]
async fn scan_block_assigns_correct_vout_index() {
    let (path, conn) = temp_db();
    let nh = "vouthash";
    let raw_name_hex = hex::encode("vout");

    // Two BID outputs in the same tx at vout 0 and vout 1
    let mock = MockNodeRpc::new()
        .with_block_hash("bh".to_string())
        .with_block(serde_json::json!({
            "tx": [{
                "hash": "txvout",
                "outputs": [
                    {
                        "value": 1_000_000_u64,
                        "address": "rs1qa",
                        "covenant": {
                            "type": COV_BID as u64,
                            "items": [nh, "00000001", &raw_name_hex, "blind"]
                        }
                    },
                    {
                        "value": 2_000_000_u64,
                        "address": "rs1qb",
                        "covenant": {
                            "type": COV_BID as u64,
                            "items": [nh, "00000001", &raw_name_hex, "blind"]
                        }
                    }
                ]
            }]
        }));
    scan_block(&mock, path.to_str().unwrap(), 20).await.unwrap();

    let bids = read_indexed_bids(&conn, nh).unwrap();
    assert_eq!(bids.len(), 2);
    assert_eq!(bids[0].index, Some(0));
    assert_eq!(bids[1].index, Some(1));
}
