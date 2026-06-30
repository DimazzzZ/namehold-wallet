-- Non-custodial chain cache: derived addresses, tracked UTXOs, transactions,
-- and sync cursors. Populated by the local WalletSyncEngine from node RPC /
-- explorer reads. See implementation_plan.md.

CREATE TABLE IF NOT EXISTS derived_addresses (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    wallet_profile_id   TEXT    NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    account_index       INTEGER NOT NULL,
    branch              INTEGER NOT NULL CHECK (branch IN (0, 1)),
    child_index         INTEGER NOT NULL,
    address             TEXT    NOT NULL,
    script_pubkey_hex   TEXT    NOT NULL,
    public_key_hex      TEXT    NOT NULL,
    used                INTEGER NOT NULL DEFAULT 0,
    first_seen_height   INTEGER,
    last_seen_height    INTEGER,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE(wallet_profile_id, account_index, branch, child_index)
);

CREATE INDEX IF NOT EXISTS idx_derived_addr_profile ON derived_addresses(wallet_profile_id);
CREATE INDEX IF NOT EXISTS idx_derived_addr_addr ON derived_addresses(address);

CREATE TABLE IF NOT EXISTS tracked_utxos (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    txid                TEXT    NOT NULL,
    vout                INTEGER NOT NULL,
    wallet_profile_id   TEXT    NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    address             TEXT    NOT NULL,
    script_pubkey_hex   TEXT    NOT NULL,
    value_doos          INTEGER NOT NULL,
    height              INTEGER,
    coinbase            INTEGER NOT NULL DEFAULT 0,
    covenant_type       INTEGER NOT NULL DEFAULT 0,
    covenant_json       TEXT,
    spend_class         TEXT    NOT NULL DEFAULT 'liquid_hns'
                            CHECK (spend_class IN ('liquid_hns', 'name_control', 'name_lockup', 'unsupported')),
    spent_by_txid       TEXT,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE(wallet_profile_id, txid, vout)
);

CREATE INDEX IF NOT EXISTS idx_utxo_profile ON tracked_utxos(wallet_profile_id);
CREATE INDEX IF NOT EXISTS idx_utxo_spent ON tracked_utxos(spent_by_txid);

CREATE TABLE IF NOT EXISTS wallet_transactions_cache (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    wallet_profile_id   TEXT    NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    txid                TEXT    NOT NULL,
    height              INTEGER,
    time                TEXT,
    raw_json            TEXT,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE(wallet_profile_id, txid)
);

CREATE INDEX IF NOT EXISTS idx_txcache_profile ON wallet_transactions_cache(wallet_profile_id);

CREATE TABLE IF NOT EXISTS sync_cursors (
    wallet_profile_id   TEXT    PRIMARY KEY REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    last_height         INTEGER NOT NULL DEFAULT 0,
    last_synced_at      TEXT
);
