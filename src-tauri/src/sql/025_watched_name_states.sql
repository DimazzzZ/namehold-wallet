-- Watched-name state cache written by namehold-syncd (the background daemon).
-- Purpose:
--   1. Feed the Watchlist page with fresh Countdown / Highest-bid / Expiry
--      columns on first open (no per-name RPC round-trip needed).
--   2. Give the daemon a place to diff the last-seen phase against the
--      current phase so it can emit OS notifications on transitions
--      (->BIDDING, name re-opened / available, bidding-soon lead-time,
--      highest-bid crossed threshold).
--
-- Global table (not per wallet profile): watched_names is itself global.
-- The daemon is the SOLE writer; the app only reads it (via
-- get_watched_states). WAL + busy_timeout=5000ms (set in db::connection::open)
-- handles the app<->daemon concurrent-read case safely.
CREATE TABLE IF NOT EXISTS watched_name_states (
    name              TEXT PRIMARY KEY,
    last_phase        TEXT,          -- 'OPENING'|'BIDDING'|'REVEAL'|'CLOSED'|NULL
    last_state_json   TEXT,          -- full HsdName snapshot as JSON (UI cache)
    last_highest_doos INTEGER,       -- last-seen highest bid, doos (nullable)
    blocks_until_next INTEGER,       -- min(stats.blocksUntil*) at last poll,
                                     -- used by the daemon to skip far-future names
    polled_at         TEXT NOT NULL, -- ISO8601 UTC of last successful poll
    updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_watched_name_states_polled
    ON watched_name_states(polled_at);
