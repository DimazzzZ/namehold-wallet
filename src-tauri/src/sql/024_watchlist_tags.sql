-- Watchlist tags: add a `tags` column to the watched_names table so users
-- can group / filter their tracked names (e.g. "auctions", "expiring-soon",
-- "competitors"). Stored as a comma-separated list of trimmed tag strings —
-- kept as free text so we don't need a join table for a small, per-user set.
--
-- SQLite requires a constant default; NULL would work but empty-string is
-- friendlier when reading back into a `String` field on the Rust side.
ALTER TABLE watched_names ADD COLUMN tags TEXT NOT NULL DEFAULT '';
