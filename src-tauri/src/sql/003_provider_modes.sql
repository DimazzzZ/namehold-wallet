-- Provider/connection-mode settings for multi-provider read architecture.
-- See implementation_plan.md for the full design.

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('connection_mode',               'local_managed_hsd'),
    ('external_read_provider',        'none'),
    ('external_read_api_url',         'https://e.hnsfans.com'),
    ('external_read_watch_addresses', '[]'),
    ('external_read_watch_names',     '[]'),
    ('remote_hsd_label',              ''),
    ('trusted_remote_hsd',            'false'),
    ('future_signer_mode',            'none');
