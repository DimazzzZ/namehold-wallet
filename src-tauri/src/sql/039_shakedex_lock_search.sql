-- R34: whether this profile's own Shakedex locks were searched for in its
-- address history after a restore. Stamped only once every derived address
-- was read from an authoritative node and every candidate was judged, so a
-- partial or failed search never counts as done. Existing profiles start NULL
-- (not done) and get one search the first time the unlocked wallet is used.
ALTER TABLE wallet_profiles ADD COLUMN shakedex_lock_search_done_at TEXT;
