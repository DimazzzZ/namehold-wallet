-- Name watchlist: track names you don't own for monitoring purposes.
-- Used by the Watchlist page to show names of interest (auction monitoring,
-- competitor tracking, expiry alerts).
CREATE TABLE IF NOT EXISTS watched_names (
    name TEXT PRIMARY KEY,
    added_at TEXT NOT NULL DEFAULT (datetime('now')),
    notes TEXT DEFAULT ''
);

-- Index for sorting by addition time (newest first).
CREATE INDEX IF NOT EXISTS idx_watched_names_added_at
    ON watched_names(added_at DESC);
