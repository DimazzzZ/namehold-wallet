-- Persist the resolved owner address for a tracked name alongside the owner
-- outpoint (owner_txid/owner_vout). This lets later reconciliation logic and
-- the frontend read the address without re-hitting the explorer API.

ALTER TABLE tracked_name_states ADD COLUMN owner_address TEXT;
