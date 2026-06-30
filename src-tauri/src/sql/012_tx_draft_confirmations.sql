-- Track on-chain confirmation of broadcast transaction drafts.
--
-- A draft was previously marked 'broadcasted' the moment the node accepted it to
-- the mempool and never moved again — so a tx that was broadcast but never mined
-- (e.g. an auction bid that misses its bidding window) looked successful forever.
-- This adds two terminal statuses, 'confirmed' (mined, >=1 confirmation) and
-- 'dropped' (evicted/never confirmed past a grace window), plus a confirmation
-- height. SQLite cannot ALTER a CHECK constraint in place, so the table is
-- recreated and its rows copied. Nothing references wallet_tx_drafts, so the
-- drop/rename is safe; the FK to wallet_profiles is re-declared on the new table.

CREATE TABLE wallet_tx_drafts_new (
    id                  TEXT    PRIMARY KEY,
    wallet_profile_id   TEXT    NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    action              TEXT    NOT NULL,
    unsigned_tx_hex     TEXT    NOT NULL,
    signed_tx_hex       TEXT,
    signing_inputs_json TEXT    NOT NULL,
    summary_json        TEXT    NOT NULL,
    status              TEXT    NOT NULL DEFAULT 'draft'
                            CHECK (status IN ('draft','signed','broadcast_pending','broadcasted','confirmed','dropped','failed')),
    error_message       TEXT,
    txid                TEXT,
    confirmation_height INTEGER,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO wallet_tx_drafts_new
    (id, wallet_profile_id, action, unsigned_tx_hex, signed_tx_hex,
     signing_inputs_json, summary_json, status, error_message, txid,
     confirmation_height, created_at, updated_at)
SELECT
    id, wallet_profile_id, action, unsigned_tx_hex, signed_tx_hex,
    signing_inputs_json, summary_json, status, error_message, txid,
    NULL, created_at, updated_at
FROM wallet_tx_drafts;

DROP TABLE wallet_tx_drafts;
ALTER TABLE wallet_tx_drafts_new RENAME TO wallet_tx_drafts;

CREATE INDEX IF NOT EXISTS idx_tx_drafts_profile ON wallet_tx_drafts(wallet_profile_id);
CREATE INDEX IF NOT EXISTS idx_tx_drafts_status ON wallet_tx_drafts(status);
