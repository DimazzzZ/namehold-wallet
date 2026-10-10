-- T6: the market jobs' bookkeeping (R23, R25). market_status (034) is
-- queries::MarketStatus by its spelling; market_retry_at (034) is when the
-- next market action on the listing is due (RFC 3339, UTC); market_attempts
-- counts the failed tries since the last success, the exponent of the
-- backoff; market_error is the market's own refusal, or why there was no
-- answer; market_accepted is 1 once the market accepted an upload of the
-- listing (or served our own copy back), the only case in which a Cancelling
-- listing is kept on the market; market_changed is 1 while what is sent has
-- changed since then (a market copy that differs is our own older one).
-- A move into Listed from another state clears all six
-- (ListingWrite::market_reset_sql); a Lower price or an expiry refresh makes
-- a told listing Retrying, due now, and sets market_changed, keeping
-- market_accepted; a reorg back to an unsent cancel does the same
-- (queries::mark_listing_cancel_unmined); an accepted upload or a matched
-- copy sets market_accepted and clears market_changed. market_told is 1
-- once the market took something of ours for this listing (its pending post
-- accepted, an upload accepted, our copy served back): the listing is then
-- reported when its cancel or sale is mined (R28). Sticky: no reset, change,
-- reorg or failure clears it, since a listing row is one lock (a new lock is
-- a new row).
ALTER TABLE shakedex_listings ADD COLUMN market_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE shakedex_listings ADD COLUMN market_error TEXT;
ALTER TABLE shakedex_listings ADD COLUMN market_accepted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE shakedex_listings ADD COLUMN market_changed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE shakedex_listings ADD COLUMN market_told INTEGER NOT NULL DEFAULT 0;
