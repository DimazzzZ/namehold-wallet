-- T3: the FINALIZE into the lock built by Finalize & sign. Its txid is the
-- listing's lock_txid (output 0); the draft is kept to tell an unsent or
-- refused FINALIZE from one that may still land.
ALTER TABLE shakedex_listings ADD COLUMN lock_finalize_draft_id TEXT;
