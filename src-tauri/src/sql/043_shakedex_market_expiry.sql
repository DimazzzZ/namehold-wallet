-- The expiry LearnHNS Market holds for a listing, as its copy last read showed.
-- It can differ from the one in our own file: since the market's PR #2 an
-- upload never moves it, only a renewal signed with the lock key does.
CREATE TABLE IF NOT EXISTS shakedex_market_expiry (
    listing_id  TEXT    PRIMARY KEY
                    REFERENCES shakedex_listings(id) ON DELETE CASCADE,
    expires_at  INTEGER NOT NULL,
    read_at     INTEGER NOT NULL
);
