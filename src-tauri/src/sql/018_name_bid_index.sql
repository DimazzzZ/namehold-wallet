-- Chain-scanner index: per-name BID/REVEAL covenant outputs from the full chain.
-- NOT wallet-scoped — this is a public chain index (any bidder, any name).
-- Populated by the background chain scanner (`commands::chain_scan`).

CREATE TABLE IF NOT EXISTS name_bid_outpoints (
    bid_txid          TEXT    NOT NULL,
    bid_vout          INTEGER NOT NULL,
    name_hash_hex     TEXT    NOT NULL,
    name              TEXT,
    lockup_value_doos INTEGER NOT NULL,
    address           TEXT,
    height            INTEGER NOT NULL,
    reveal_txid       TEXT,
    reveal_value_doos INTEGER,
    PRIMARY KEY (bid_txid, bid_vout)
);

CREATE INDEX IF NOT EXISTS idx_name_bid_outpoints_name_hash
    ON name_bid_outpoints (name_hash_hex);

-- Global chain-scanner cursor: a singleton row tracking how far the scanner has
-- advanced. Independent of any wallet profile (the two existing cursors in
-- sync_cursors / wallet_profiles are profile-scoped and unsuitable).
CREATE TABLE IF NOT EXISTS chain_scan_cursor (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    last_height INTEGER NOT NULL DEFAULT 0
);

INSERT OR IGNORE INTO chain_scan_cursor (id, last_height) VALUES (1, 0);
