-- Remove obsolete custodial-wallet and connection-mode settings. Split the
-- retired backend name so it cannot be mistaken for a supported integration.
DELETE FROM settings WHERE key IN (
    'h' || 'sd_wallet_api_url',
    'h' || 'sd_node_api_url',
    'h' || 'sd_api_key',
    'h' || 'sd_wallet_id',
    'h' || 'sd_network',
    'h' || 'sd_prefix',
    'write_mode',
    'connection_mode',
    'external_read_provider',
    'external_read_api_url',
    'external_read_watch_addresses',
    'external_read_watch_names',
    'remote_' || 'h' || 'sd_label',
    'trusted_remote_' || 'h' || 'sd',
    'future_signer_mode'
);
