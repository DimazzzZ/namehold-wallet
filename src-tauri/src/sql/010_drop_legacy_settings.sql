-- Remove legacy hsd-wallet / connection-mode settings. The wallet is now a
-- single non-custodial model: reads via the explorer (explorer_api_url),
-- sending via one node (node_rpc_url). Keys not deleted here keep their values.
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
    'chain_source',
    'allow_remote_broadcast',
    'custody_mode'
);
