-- T6: the market jobs' bookkeeping (R23, R25). market_status (034) is
-- queries::MarketStatus by its spelling; market_retry_at (034) is when the
-- next market action on the listing is due (RFC 3339, UTC); market_attempts
-- counts the failed tries since the last success, the exponent of the
-- backoff; market_error is the market's own refusal, or why there was no
-- answer. A write that may put a listing back on the market set clears all
-- four (ListingWrite::market_reset_sql).
ALTER TABLE shakedex_listings ADD COLUMN market_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE shakedex_listings ADD COLUMN market_error TEXT;
