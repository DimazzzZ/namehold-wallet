-- Tracked name states, bid commitments, and transaction drafts for the
-- non-custodial wallet engine. See implementation_plan.md.

CREATE TABLE IF NOT EXISTS tracked_name_states (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    wallet_profile_id   TEXT    NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    name                TEXT    NOT NULL,
    name_hash_hex       TEXT    NOT NULL,
    state               TEXT    NOT NULL,
    owner_txid          TEXT,
    owner_vout          INTEGER,
    height              INTEGER,
    renewal_height      INTEGER,
    transfer_height     INTEGER,
    renewals            INTEGER,
    reserved            INTEGER,
    weak                INTEGER,
    raw_json            TEXT,
    updated_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE(wallet_profile_id, name)
);

CREATE INDEX IF NOT EXISTS idx_name_state_profile ON tracked_name_states(wallet_profile_id);
CREATE INDEX IF NOT EXISTS idx_name_state_hash ON tracked_name_states(name_hash_hex);

-- Blind/nonce material is SECRET wallet state. Never expose to the frontend.
CREATE TABLE IF NOT EXISTS bid_commitments (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    wallet_profile_id   TEXT    NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    name                TEXT    NOT NULL,
    name_hash_hex       TEXT    NOT NULL,
    address             TEXT    NOT NULL,
    branch              INTEGER NOT NULL,
    child_index         INTEGER NOT NULL,
    bid_value_doos      INTEGER NOT NULL,
    lockup_value_doos   INTEGER NOT NULL,
    nonce_hex           TEXT    NOT NULL,
    blind_hex           TEXT    NOT NULL,
    bid_txid            TEXT,
    reveal_txid         TEXT,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE(wallet_profile_id, name, blind_hex)
);

CREATE INDEX IF NOT EXISTS idx_bid_profile ON bid_commitments(wallet_profile_id);

CREATE TABLE IF NOT EXISTS wallet_tx_drafts (
    id                  TEXT    PRIMARY KEY,
    wallet_profile_id   TEXT    NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    action              TEXT    NOT NULL,
    unsigned_tx_hex     TEXT    NOT NULL,
    signed_tx_hex       TEXT,
    signing_inputs_json TEXT    NOT NULL,
    summary_json        TEXT    NOT NULL,
    status              TEXT    NOT NULL DEFAULT 'draft'
                            CHECK (status IN ('draft','signed','broadcast_pending','broadcasted','failed')),
    error_message       TEXT,
    txid                TEXT,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_tx_drafts_profile ON wallet_tx_drafts(wallet_profile_id);
CREATE INDEX IF NOT EXISTS idx_tx_drafts_status ON wallet_tx_drafts(status);
