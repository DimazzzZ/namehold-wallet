-- Per-profile node configuration overrides (ADR-001).
--
-- A profile may override the global node configuration with its own
-- `node_rpc_url`, `node_rpc_api_key`, and `chain_source`. Resolution order is
-- per-profile override -> global `settings` -> built-in default; see
-- `effective_node_config_for_profile`. A key that is absent here means "no
-- override for this key" and resolution falls through to global/default; a key
-- present with a value is an explicit choice that is never silently abandoned.
--
-- Key/value shape mirrors the global `settings` table so the resolver reads
-- both with the same accessor. Rows are scoped to a profile and cascade-delete
-- with it, so deleting a profile cannot leave orphaned overrides behind.
CREATE TABLE IF NOT EXISTS profile_settings (
    profile_id  TEXT NOT NULL REFERENCES wallet_profiles(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    PRIMARY KEY (profile_id, key)
);
