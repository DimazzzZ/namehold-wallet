-- Remove legacy hsd-wallet / connection-mode settings. The wallet is now a
-- single non-custodial model: reads via the explorer (explorer_api_url),
-- sending via one node (node_rpc_url). Keys not deleted here keep their values.
--
-- `chain_source` and `allow_remote_broadcast` were once listed here, then
-- reintroduced as live settings (remote-node support); they are no longer
-- deleted.
--
-- `hsd_prefix` IS still deleted here and was also reintroduced, by 011, which
-- re-seeds it empty. On a database upgrading across this point that costs the
-- user their configured hsd data directory: 011's `INSERT OR IGNORE` puts back
-- an empty value, not theirs, so the app falls back to `~/.hsd` and the chain
-- appears to have vanished. Left as it is on purpose — a migration is a record
-- of a transformation that already ran, and editing a shipped one would change
-- history for databases that have not reached it while doing nothing for those
-- that have. Anyone re-seeding a setting a later migration restores should
-- carry the old value across instead of dropping it.
DELETE FROM settings WHERE key IN (
    'hsd_wallet_api_url',
    'hsd_node_api_url',
    'hsd_api_key',
    'hsd_wallet_id',
    'hsd_network',
    'hsd_prefix',
    'write_mode',
    'connection_mode',
    'external_read_provider',
    'external_read_api_url',
    'external_read_watch_addresses',
    'external_read_watch_names',
    'remote_hsd_label',
    'trusted_remote_hsd',
    'future_signer_mode',
    'custody_mode'
);
