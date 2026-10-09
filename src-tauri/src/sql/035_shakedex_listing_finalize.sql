-- T3: the FINALIZE into the lock built by Finalize & sign. Its txid is the
-- listing's lock_txid, and lock_vout the index of its FINALIZE output into
-- the lock (sell::lock_output); the draft is kept to tell an unsent or
-- refused FINALIZE from one that may still land.
ALTER TABLE shakedex_listings ADD COLUMN lock_finalize_draft_id TEXT;
