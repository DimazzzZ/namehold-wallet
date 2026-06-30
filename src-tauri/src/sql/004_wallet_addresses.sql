-- Per-wallet address cache.
--
-- Stores the set of public addresses known to belong to a specific wallet
-- (identified by the hsd wallet id). This lets external read-only mode
-- automatically resolve the *selected wallet's* full balance/assets without
-- the user having to enter watch addresses manually.
--
-- Addresses are populated during a sync against a (local or remote) hsd by
-- inspecting the wallet's coins and transaction history. Each row is scoped to
-- a wallet so switching the selected wallet shows the right data.

CREATE TABLE IF NOT EXISTS wallet_addresses (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    wallet_id   TEXT NOT NULL,
    address     TEXT NOT NULL,
    branch      INTEGER,          -- 0 = receive, 1 = change (when known)
    first_seen  TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen   TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(wallet_id, address)
);

CREATE INDEX IF NOT EXISTS idx_wallet_addresses_wallet
    ON wallet_addresses(wallet_id);
