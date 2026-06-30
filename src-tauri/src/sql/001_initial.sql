CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS assets (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    tld               TEXT    NOT NULL UNIQUE,
    status            TEXT    NOT NULL DEFAULT 'not_started'
                          CHECK (status IN (
                              'not_started',
                              'namebase_transfer_requested',
                              'waiting_transfer_tx',
                              'transfer_seen_on_chain',
                              'waiting_finalize',
                              'finalized_owned',
                              'failed_or_stuck',
                              'do_not_touch_staked'
                          )),
    is_staked         INTEGER NOT NULL DEFAULT 0,
    category          TEXT,
    tags              TEXT,
    notes             TEXT,
    hns_received      INTEGER,
    transfer_tx_hash  TEXT,
    finalize_tx_hash  TEXT,
    name_state        TEXT,
    expires_at_height INTEGER,
    days_until_expire REAL,
    last_synced_at    TEXT,
    created_at        TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_assets_status ON assets(status);
CREATE INDEX IF NOT EXISTS idx_assets_staked ON assets(is_staked);
CREATE INDEX IF NOT EXISTS idx_assets_tld    ON assets(tld);

CREATE TABLE IF NOT EXISTS batches (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    description TEXT,
    status      TEXT    NOT NULL DEFAULT 'planned'
                    CHECK (status IN ('planned', 'in_progress', 'completed', 'paused', 'cancelled')),
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS batch_assets (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id   INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
    asset_id   INTEGER NOT NULL REFERENCES assets(id)  ON DELETE CASCADE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    UNIQUE(batch_id, asset_id)
);

CREATE TABLE IF NOT EXISTS wallet_snapshots (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    wallet_name  TEXT    NOT NULL,
    balance      INTEGER NOT NULL,
    address      TEXT,
    name_count   INTEGER NOT NULL,
    raw_json     TEXT
);

CREATE TABLE IF NOT EXISTS audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp  TEXT    NOT NULL DEFAULT (datetime('now')),
    action     TEXT    NOT NULL,
    entity     TEXT,
    entity_id  INTEGER,
    detail     TEXT,
    created_at TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_log(action);
CREATE INDEX IF NOT EXISTS idx_audit_entity ON audit_log(entity, entity_id);

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('hsd_wallet_api_url', 'http://127.0.0.1:12039'),
    ('hsd_node_api_url',   'http://127.0.0.1:12037'),
    ('hsd_api_key',        ''),
    ('hsd_wallet_id',      'primary'),
    ('hsd_network',        'mainnet'),
    ('write_mode',         'false');
