-- Upgrade existing installations to the authenticated wallet RPC v1 backend.
-- Preserve a user-selected chain directory from the retired backend, while
-- intentionally not reusing its incompatible authentication credential.
INSERT INTO settings (key, value)
SELECT 'hsrd_data_dir', value
FROM settings
WHERE key = 'h' || 'sd_data_dir' AND value <> ''
ON CONFLICT(key) DO UPDATE SET value = CASE
    WHEN settings.value = '' THEN excluded.value
    ELSE settings.value
END;

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('chain_source',       'managed_sidecar'),
    ('hsrd_rpc_url',       'http://127.0.0.1:12037'),
    ('hsrd_authorization', ''),
    ('hsrd_network',       'main'),
    ('hsrd_data_dir',      ''),
    ('hsrd_path',          ''),
    ('autostart_hsrd',     'true'),
    ('custody_mode',       'noncustodial_local'),
    ('allow_remote_broadcast', 'false');

DELETE FROM settings WHERE key IN (
    'h' || 'sd_wallet_api_url',
    'h' || 'sd_node_api_url',
    'h' || 'sd_api_key',
    'h' || 'sd_wallet_id',
    'h' || 'sd_network',
    'h' || 'sd_prefix',
    'h' || 'sd_data_dir',
    'h' || 'sd_path',
    'autostart_' || 'h' || 'sd',
    'node_rpc_url',
    'node_rpc_api_key'
);
