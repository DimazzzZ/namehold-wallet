-- Settings for the non-custodial signing engine. These control how the local
-- wallet engine talks to a node for broadcast/reads and which active wallet
-- profile is selected. See implementation_plan.md.

INSERT OR IGNORE INTO settings (key, value) VALUES
    -- Selected non-custodial wallet profile id (empty = none selected).
    ('active_wallet_profile_id',        ''),
    -- Where to broadcast signed transactions and read chain state from.
    -- 'local_node' = managed/local hsd node RPC; 'remote_node' = user-provided
    -- node RPC; 'explorer' = read-only explorer (broadcast disabled).
    ('chain_source',                    'local_node'),
    ('node_rpc_url',                    'http://127.0.0.1:12037'),
    ('node_rpc_api_key',                ''),
    ('explorer_api_url',                'https://e.hnsfans.com'),
    -- Custody model: 'noncustodial_local' (Namehold holds keys, signs locally)
    -- or 'legacy_hsd_wallet' (deprecated: hsd wallet holds keys and signs).
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
