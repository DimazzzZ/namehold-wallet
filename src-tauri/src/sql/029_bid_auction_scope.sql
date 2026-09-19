-- Tie every indexed BID to the auction it was placed in.
--
-- `name_bid_outpoints` was keyed by (network, nameHash), which is the name, not
-- the auction. A name can be auctioned many times — an auction nobody reveals
-- in simply lapses and the name becomes available again — so every BID ever
-- seen for a name piled up under one key and `read_name_bids` served all of
-- them at once. A name sitting at "Waiting for Bidding" with no auction open at
-- all still listed the bids from its previous, lapsed auction.
--
-- The BID covenant already carries the auction's identity: item[1] is the
-- u32 OPEN height of the auction being bid on (`covenants::bid`'s `start`,
-- hsd `lib/wallet/wallet.js`). Record it and the index can answer "bids in THIS
-- auction" instead of "bids for this name, ever".
--
-- Existing rows predate the column and their auction cannot be recovered from
-- what was stored, so the table is rebuilt empty and the cursors reset — the
-- scanner refills both from the chain.

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

CREATE INDEX IF NOT EXISTS idx_name_bid_outpoints_name_hash
    ON name_bid_outpoints (network, name_hash_hex, name_start_height);

UPDATE chain_scan_cursor SET last_height = 0;
