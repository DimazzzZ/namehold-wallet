-- Shakedex listings: seller-side tracking of a name locked for sale. A
-- listing is keyed by name and its lock outpoint, never by lock address (two
-- names may share a lock key, ADR 0004). Day 0 (T2) writes a 'locking' row
-- with the lock TRANSFER draft and the reserved payment and cancel addresses;
-- later stages set the lock outpoint, the signed steps and the listing file.
CREATE TABLE IF NOT EXISTS shakedex_listings (
    id TEXT PRIMARY KEY,
    wallet_profile_id TEXT NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('buy_now','reverse_auction')),
    state TEXT NOT NULL DEFAULT 'locking'
        CHECK (state IN ('locking','ready_to_finalize','finalizing','listed','sale_pending',
                         'sold','cancelling','cancel_awaiting_finalize','cancel_finalizing',
                         'cancelled','aborted','restored','expired')),
    lock_pubkey_hex TEXT NOT NULL,
    lock_transfer_draft_id TEXT,
    lock_transfer_txid TEXT,
    lock_txid TEXT,
    lock_vout INTEGER,
    payment_address TEXT,
    cancel_address TEXT,
    -- Receive-branch index of cancel_address (the cancel's lock input carries it, T5).
    cancel_child_index INTEGER,
    steps_json TEXT NOT NULL DEFAULT '[]',
    listing_file_json TEXT,
    publish INTEGER NOT NULL DEFAULT 0 CHECK (publish IN (0, 1)),
    market_status TEXT,
    market_retry_at TEXT,
    -- Unix seconds: the listing file's expiresAt (spec R23, MTP + 365 days).
    expires_at INTEGER,
    -- The Cancel transfer draft that withdraws a listing still locking, and
    -- its txid, which outlives the draft (a dropped cancel may still be mined).
    abort_draft_id TEXT,
    abort_txid TEXT,
    sold_txid TEXT,
    cancel_txid TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_shakedex_listings_profile_name
    ON shakedex_listings(wallet_profile_id, name, state);

-- A lock coin is tracked by one listing.
CREATE UNIQUE INDEX IF NOT EXISTS idx_shakedex_listings_lock_outpoint
    ON shakedex_listings(wallet_profile_id, name, lock_txid, lock_vout)
    WHERE lock_txid IS NOT NULL;

-- At most one open listing per name: the terminal states are
-- ListingState::TERMINAL (db/queries.rs).
CREATE UNIQUE INDEX IF NOT EXISTS idx_shakedex_listings_open_name
    ON shakedex_listings(wallet_profile_id, name)
    WHERE state NOT IN ('sold','cancelled','aborted','expired');
