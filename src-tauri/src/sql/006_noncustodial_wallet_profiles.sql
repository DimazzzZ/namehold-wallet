-- Non-custodial wallet profiles, encrypted secrets, and HD accounts.
-- See implementation_plan.md for the full design.
--
-- Namehold owns key custody locally. Secrets (mnemonic-derived seed / xpriv)
-- are stored only in encrypted form (Argon2id + AES-256-GCM). Watch-only
-- profiles store no secret at all.

CREATE TABLE IF NOT EXISTS wallet_profiles (
    id                  TEXT    PRIMARY KEY,
    label               TEXT    NOT NULL,
    kind                TEXT    NOT NULL
                            CHECK (kind IN ('mnemonic_hot', 'xpriv_hot', 'watch_only_xpub')),
    network             TEXT    NOT NULL
                            CHECK (network IN ('mainnet', 'testnet', 'regtest')),
    account_xpub        TEXT    NOT NULL,
    account_index       INTEGER NOT NULL DEFAULT 0,
    receive_depth       INTEGER NOT NULL DEFAULT 0,
    change_depth        INTEGER NOT NULL DEFAULT 0,
    receive_address     TEXT,
    last_synced_height  INTEGER,
    last_synced_at      TEXT,
    watch_only          INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_wallet_profiles_kind ON wallet_profiles(kind);

-- Encrypted secret envelopes. One row per hot wallet profile. Watch-only
-- profiles never have a row here.
CREATE TABLE IF NOT EXISTS wallet_secrets (
    wallet_profile_id   TEXT    PRIMARY KEY REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    version             INTEGER NOT NULL DEFAULT 1,
    kdf                 TEXT    NOT NULL DEFAULT 'argon2id',
    kdf_salt_hex        TEXT    NOT NULL,
    kdf_memory_kib      INTEGER NOT NULL DEFAULT 65536,
    kdf_iterations      INTEGER NOT NULL DEFAULT 3,
    kdf_parallelism     INTEGER NOT NULL DEFAULT 1,
    cipher              TEXT    NOT NULL DEFAULT 'aes-256-gcm',
    nonce_hex           TEXT    NOT NULL,
    ciphertext_hex      TEXT    NOT NULL,
    public_fingerprint  TEXT    NOT NULL,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);
