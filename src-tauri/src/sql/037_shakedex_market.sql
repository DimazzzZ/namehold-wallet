-- T6: the market jobs' bookkeeping (R23, R25). market_status (034) is
-- queries::MarketStatus by its spelling; market_retry_at (034) is when the
-- next market action on the listing is due (RFC 3339, UTC); market_attempts
-- counts the failed tries since the last success, the exponent of the
-- backoff; market_error is the market's own refusal, or why there was no
-- answer; market_accepted is 1 once the market accepted an upload of the
-- listing (or served our own copy back), the only case in which a Cancelling
-- listing is kept on the market. A write that may put a listing back on the
-- market set clears all five (ListingWrite::market_reset_sql); a reorg back
-- to an unsent cancel keeps market_accepted (queries::mark_listing_cancel_unmined).
ALTER TABLE shakedex_listings ADD COLUMN market_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE shakedex_listings ADD COLUMN market_error TEXT;
ALTER TABLE shakedex_listings ADD COLUMN market_accepted INTEGER NOT NULL DEFAULT 0;
