-- T8: the step of ours the market was last sent (an upload, accepted or
-- not) or found holding (our copy served back): its signature, hex
-- (R24). A reverse auction's current step moves with time; the advance job
-- (shakedex_jobs::advance_reverse_auctions_step) uploads the current one
-- when it is not this. Cleared with the rest of the bookkeeping by a move
-- into Listed (ListingWrite::market_reset_sql); every other write keeps it.
ALTER TABLE shakedex_listings ADD COLUMN market_sent_step TEXT;
