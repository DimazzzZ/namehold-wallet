-- R38: a listing another device or profile of the same recovery phrase
-- manages. `managed_elsewhere_at` is when this device first saw, on LearnHNS
-- Market, a price step signed by the listing's lock key over its lock coin
-- that it never stored (a foreign step, R36); `foreign_step_json` is the
-- step seen (price, lock time, signature, the address it pays). Not market
-- bookkeeping: a write back onto the market does not clear it; only a
-- take-over does (R44, R45). Existing listings start NULL (not marked).
ALTER TABLE shakedex_listings ADD COLUMN managed_elsewhere_at TEXT;
ALTER TABLE shakedex_listings ADD COLUMN foreign_step_json TEXT;
