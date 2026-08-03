-- Settings for the non-custodial signing engine and authenticated sidecar.

INSERT OR IGNORE INTO settings (key, value) VALUES
    -- Selected non-custodial wallet profile id (empty = none selected).
    ('active_wallet_profile_id',        ''),
    -- The sidecar may be managed locally or supplied at a remote HTTPS URL.
    ('chain_source',                    'managed_sidecar'),
    ('hsrd_rpc_url',                    'http://127.0.0.1:12037'),
    -- Exact Authorization header value expected by wallet RPC v1.
    ('hsrd_authorization',              ''),
    ('hsrd_network',                    'main'),
    ('hsrd_path',                       ''),
    ('autostart_hsrd',                  'true'),
    ('explorer_api_url',                'https://e.hnsfans.com'),
    -- Namehold always owns keys and signs locally.
    ('custody_mode',                    'noncustodial_local'),
    -- Allow broadcasting locally-signed transactions to a remote provider.
    ('allow_remote_broadcast',          'false'),
    -- Address gap limit for scanning derived addresses.
    ('address_gap_limit',               '20'),
    -- Signer session timeout (seconds) before in-memory keys are zeroized.
    ('signer_session_timeout_seconds',  '900'),
    -- How Rust-owned secret ingress is presented to the user.
    ('secure_secret_entry_mode',        'native_window'),
    -- Default fee rate in doos per kvB for draft construction.
    ('fee_rate_doos_per_kvb',           '1000');
