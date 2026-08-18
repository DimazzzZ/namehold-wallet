-- Add support for Ledger hardware wallet profiles.
--
-- A new profile kind 'ledger_hardware' represents a wallet whose private keys
-- live on a Ledger Nano S/X device (official handshake-org/ledger-app-hns app).
-- The wallet acts as a transaction composer and signer orchestrator; the device
-- signs via APDU. Ledger profiles store the account xpub (public) but never have
-- a wallet_secrets row (no encrypted vault; keys never leave the device).
--
-- SQLite cannot ALTER a CHECK constraint in place, so wallet_profiles is
-- recreated with the new kind added to the CHECK list.
--
-- SAFETY (FK): Foreign keys MUST be disabled around the DROP or ON DELETE
-- CASCADE will wipe every child table (derived_addresses, tracked_utxos,
-- tracked_name_states, wallet_secrets). `PRAGMA foreign_keys` is a no-op
-- inside a transaction, so it must be toggled in autocommit mode, strictly
-- outside the BEGIN/COMMIT below.
--
-- SAFETY (atomicity): the DROP/CREATE/RENAME dance below runs inside an
-- explicit transaction so a crash or error partway through leaves either the
-- old `wallet_profiles` intact or the new one fully in place — never neither.
-- `DROP TABLE IF EXISTS wallet_profiles_new` makes a retry (after a crash
-- that got far enough to create the scratch table but not commit) safe to
-- run from scratch: DDL is transactional in SQLite, so an uncommitted CREATE
-- TABLE is rolled back with the rest of the transaction on restart.

PRAGMA foreign_keys = OFF;

BEGIN;

DROP TABLE IF EXISTS wallet_profiles_new;

CREATE TABLE wallet_profiles_new (
    id                  TEXT    PRIMARY KEY,
    label               TEXT    NOT NULL,
    kind                TEXT    NOT NULL
                            CHECK (kind IN ('mnemonic_hot', 'xpriv_hot', 'watch_only_xpub', 'ledger_hardware')),
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
    last_explorer_sync_at TEXT,
    created_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO wallet_profiles_new
    (id, label, kind, network, account_xpub, account_index, receive_depth,
     change_depth, receive_address, last_synced_height, last_synced_at,
     watch_only, last_explorer_sync_at, created_at, updated_at)
SELECT
    id, label, kind, network, account_xpub, account_index, receive_depth,
    change_depth, receive_address, last_synced_height, last_synced_at,
    watch_only, last_explorer_sync_at, created_at, updated_at
FROM wallet_profiles;

DROP TABLE wallet_profiles;
ALTER TABLE wallet_profiles_new RENAME TO wallet_profiles;

CREATE INDEX IF NOT EXISTS idx_wallet_profiles_kind ON wallet_profiles(kind);

COMMIT;

PRAGMA foreign_keys = ON;
