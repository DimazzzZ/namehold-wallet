-- Scope the chain scanner to a network, and discard everything it recorded
-- before this migration.
--
-- 018 made both the cursor and the index global. That was wrong on two counts:
--
--   1. `chain_scan_cursor` was a singleton row. Switching the active profile
--      from mainnet to regtest left a mainnet height (e.g. 347186) in it, so
--      `cursor >= tip` held against the regtest tip (e.g. 118) and the scanner
--      silently never indexed a single regtest block. `read_name_bids` then
--      read that same stale cursor as proof of coverage and served an empty
--      index as authoritative ("No bids yet" for a name that had bids).
--
--   2. `name_bid_outpoints` was keyed by nameHash alone, and a name hashes to
--      the SAME value on every network. Once the scanner does run on regtest,
--      its BID rows would answer mainnet queries for that name.
--
-- Both tables are keyed by network now.
--
-- The cursors reset to 0 and the index is dropped rather than carried over.
-- Until the fix that ships with this migration, `scan_block` read the block
-- shape of hsd's REST API (`outputs`, a bare address string, doos) while
-- `getblock` goes through JSON-RPC and emits bitcoind's shape (`vout`, an
-- address object, HNS floats) — so no block ever parsed, and any cursor height
-- recorded so far stands for blocks that were walked but never indexed.
-- Keeping those heights would mark the chain as covered forever and leave the
-- index permanently, silently empty.

-- --- cursor: singleton -> one row per network, reset ------------------------

DROP TABLE IF EXISTS chain_scan_cursor;

CREATE TABLE chain_scan_cursor (
    network     TEXT    PRIMARY KEY
                    CHECK (network IN ('main', 'testnet', 'regtest', 'simnet')),
    last_height INTEGER NOT NULL DEFAULT 0
);

INSERT INTO chain_scan_cursor (network, last_height) VALUES
    ('main', 0), ('testnet', 0), ('regtest', 0), ('simnet', 0);

-- --- index: add network to the key, drop the (necessarily empty) old rows ---

DROP TABLE IF EXISTS name_bid_outpoints;

CREATE TABLE name_bid_outpoints (
    network           TEXT    NOT NULL
                          CHECK (network IN ('main', 'testnet', 'regtest', 'simnet')),
    bid_txid          TEXT    NOT NULL,
    bid_vout          INTEGER NOT NULL,
    name_hash_hex     TEXT    NOT NULL,
    name              TEXT,
    lockup_value_doos INTEGER NOT NULL,
    address           TEXT,
    height            INTEGER NOT NULL,
    reveal_txid       TEXT,
    reveal_value_doos INTEGER,
    PRIMARY KEY (network, bid_txid, bid_vout)
);

CREATE INDEX IF NOT EXISTS idx_name_bid_outpoints_name_hash
    ON name_bid_outpoints (network, name_hash_hex);
