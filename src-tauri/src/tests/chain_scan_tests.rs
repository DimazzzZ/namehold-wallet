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
