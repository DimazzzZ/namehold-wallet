-- The bid index held every bid in the chain's history: about 19M rows and 9 GB
-- on mainnet, in the file that also holds the wallets. It now keeps recent
-- auctions only (`chain_scan::RETAINED_BLOCKS`); older ones are read from the
-- explorer, and a wallet's own bids live in `bid_commitments`.
--
-- Everything here is public chain data that the scanner can read again. Hence
-- no zero-filling for this drop: on a table this size it would write the whole
-- table through the log. `chain_scan::reclaim_free_space` then shrinks the file.
PRAGMA secure_delete = OFF;

BEGIN IMMEDIATE;

DROP TABLE IF EXISTS name_bid_outpoints;

CREATE TABLE name_bid_outpoints (
    network           TEXT    NOT NULL
                          CHECK (network IN ('main', 'testnet', 'regtest', 'simnet')),
    bid_txid          TEXT    NOT NULL,
    bid_vout          INTEGER NOT NULL,
    name_hash_hex     TEXT    NOT NULL,
    name              TEXT,
    -- OPEN height of the auction this BID belongs to (BID covenant item[1]).
    name_start_height INTEGER NOT NULL,
    lockup_value_doos INTEGER NOT NULL,
    address           TEXT,
    height            INTEGER NOT NULL,
    reveal_txid       TEXT,
    reveal_value_doos INTEGER,
    PRIMARY KEY (network, bid_txid, bid_vout)
);

CREATE INDEX idx_name_bid_outpoints_name_hash
    ON name_bid_outpoints (network, name_hash_hex, name_start_height);

-- Finds the auctions that have aged out of the window.
CREATE INDEX idx_name_bid_outpoints_start
    ON name_bid_outpoints (network, name_start_height);

-- The first auction height the index is complete from: auctions opened earlier
-- are not in it, however far `last_height` has gone.
ALTER TABLE chain_scan_cursor ADD COLUMN first_height INTEGER NOT NULL DEFAULT 0;

UPDATE chain_scan_cursor SET last_height = 0;

COMMIT;

PRAGMA secure_delete = ON;
