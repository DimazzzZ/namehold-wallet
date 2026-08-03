-- Provider/connection-mode settings for multi-provider read architecture.
-- See implementation_plan.md for the full design.

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('connection_mode',               'local_managed_hsrd'),
    ('external_read_provider',        'none'),
    ('external_read_api_url',         'https://e.hnsfans.com'),
    ('external_read_watch_addresses', '[]'),
    ('external_read_watch_names',     '[]'),
    ('remote_hsrd_label',              ''),
    ('trusted_remote_hsrd',            'false'),
    ('future_signer_mode',            'none');
