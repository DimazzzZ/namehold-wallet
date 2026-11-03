-- Every FINALIZE into a lock whose price steps this wallet stored, signed by
-- Finalize & sign or adopted from a Restored lock's own listing file. A
-- step's signature commits to the lock coin (the FINALIZE's txid, the index
-- of its FINALIZE output and that output's value), and a FINALIZE rebuilt at
-- the same tip from the same coins and fee rate is the same transaction with
-- the same txid. A txid recorded here is never signed over again, so a step
-- from a FINALIZE that was discarded can never become valid on a later one.
-- Rows are never deleted: a listing that goes back to ReadyToFinalize, and a
-- listing row that goes away, keep their record. There is deliberately no
-- foreign key: the record outlives a deleted wallet (public lock txids and
-- listing ids, no keys), so a re-imported seed stays protected.
CREATE TABLE IF NOT EXISTS shakedex_signed_lock_txids (
    lock_txid TEXT PRIMARY KEY,
    listing_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- A listing that holds a lock outpoint already either had steps signed over
-- it here or tracks a coin already mined, which only a reorg could let a
-- FINALIZE create again; recording both costs nothing.
INSERT OR IGNORE INTO shakedex_signed_lock_txids (lock_txid, listing_id)
    SELECT lower(lock_txid), id FROM shakedex_listings WHERE lock_txid IS NOT NULL;
