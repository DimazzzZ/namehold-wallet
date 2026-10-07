-- Shakedex purchases: buyer-side tracking of a name bought from a Shakedex
-- listing. A purchase moves pending_send -> unconfirmed -> awaiting_finalize
-- -> owned, or ends in lost. The destination address is reserved for the
-- purchase and its change is held back from coin selection until the
-- purchase confirms.
CREATE TABLE IF NOT EXISTS shakedex_purchases (
    id TEXT PRIMARY KEY,
    wallet_profile_id TEXT NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    listing_json TEXT NOT NULL,
    lock_txid TEXT NOT NULL,
    lock_vout INTEGER NOT NULL,
    price_doos INTEGER NOT NULL,
    purchase_draft_id TEXT NOT NULL,
    purchase_txid TEXT NOT NULL,
    destination_address TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending_send'
        CHECK (state IN ('pending_send','unconfirmed','awaiting_finalize','owned','lost')),
    purchase_height INTEGER,
    blocks_remaining INTEGER,
    missing_since_height INTEGER,
    rebroadcast_count INTEGER NOT NULL DEFAULT 0,
    lost_reason TEXT,
    finalize_draft_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_shakedex_purchases_profile_state
    ON shakedex_purchases(wallet_profile_id, state);

-- At most one open purchase may spend a given lock outpoint.
CREATE UNIQUE INDEX IF NOT EXISTS idx_shakedex_purchases_open_lock
    ON shakedex_purchases(wallet_profile_id, lock_txid, lock_vout)
    WHERE state IN ('pending_send','unconfirmed');
