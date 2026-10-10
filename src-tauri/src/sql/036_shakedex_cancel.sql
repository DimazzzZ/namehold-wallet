-- T5: the cancel (R28). cancel_draft_id is our signed cancel draft, kept to
-- tell an unsent cancel from one that may still land; cancel_txid (034) is
-- that cancel's txid until a mined TRANSFER out of the lock committing to an
-- address of ours (ours, or another device's) replaces it with its own, and
-- cancel_vout is that TRANSFER's output; cancel_finalize_draft_id is the
-- FINALIZE that brings the name home; cancel_blocks_remaining is what was
-- left of the cancel's transfer lockup at the last sync (the reminder).
ALTER TABLE shakedex_listings ADD COLUMN cancel_draft_id TEXT;
ALTER TABLE shakedex_listings ADD COLUMN cancel_vout INTEGER;
ALTER TABLE shakedex_listings ADD COLUMN cancel_finalize_draft_id TEXT;
ALTER TABLE shakedex_listings ADD COLUMN cancel_blocks_remaining INTEGER;
