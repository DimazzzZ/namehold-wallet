/**
 * Web-QA mock backend.
 *
 * When running in a plain browser (no Tauri), every `invoke()` call is routed
 * here.  The mock returns realistic placeholder data so the full UI renders
 * and can be clicked through without crashing.
 *
 * Commands not listed in the map fall through to a console warning and return
 * `null`, which keeps the app functional (queries degrade gracefully).
 */
/* eslint-disable @typescript-eslint/no-unused-vars */

type Handler = (args?: Record<string, unknown>) => unknown;

const handlers: Record<string, Handler> = {
  // ── Settings ──────────────────────────────────────────────────────────
  get_settings: () => ({
    node_rpc_url: "http://127.0.0.1:12037",
    node_rpc_api_key: "",
    hsd_prefix: "",
    hsd_path: "",
    explorer_api_url: "https://hns.fans",
    address_gap_limit: "20",
    signer_session_timeout_seconds: "900",
    advanced_mode: "false",
    onboarding_complete: "true",
  }),

  update_setting: () => null,

  // ── Updates ───────────────────────────────────────────────────────────
  // Browser QA has no real updater; report a fixed version and "up to date".
  current_version: () => "0.2.1",
  check_for_update: () => null,
  install_update: () => null,

  // ── Wallet profiles ───────────────────────────────────────────────────
  list_wallet_profiles: () => [
    {
      id: "webqa-profile",
      label: "QA Wallet",
      kind: "mnemonic_hot",
      network: "mainnet",
      accountXpub: "xpub6C...(mock)",
      accountIndex: 0,
      receiveDepth: 5,
      changeDepth: 3,
      receiveAddress: "hs1q9g6...(mock)",
      lastSyncedHeight: 100000,
      lastSyncedAt: new Date().toISOString(),
      lastExplorerSyncAt: new Date().toISOString(),
      watchOnly: false,
      hasPassphrase: false,
      active: true,
    },
  ],

  get_signer_session: () => ({
    walletProfileId: "webqa-profile",
    unlocked: true,
    unlockedUntilEpochMs: Date.now() + 3_600_000,
  }),

  get_write_capability: () => ({
    signerUnlocked: true,
    broadcasterAvailable: true,
    canWrite: true,
    reason: null,
  }),

  // ── Balances ──────────────────────────────────────────────────────────
  get_wallet_balances: () => ({
    liquidDoos: 5_000_000_000,
    nameControlDoos: 1_200_000_000,
    nameLockupDoos: 800_000_000,
    totalDoos: 7_000_000_000,
  }),

  read_balance: () => ({
    confirmed: 5_000_000_000,
    unconfirmed: 0,
    locked_unconfirmed: 0,
    locked_confirmed: 800_000_000,
  }),

  // ── Names ─────────────────────────────────────────────────────────────
  read_names: () => [
    {
      name: "example",
      state: "CLOSED",
      height: 50000,
      renewal: 100000,
      owner: { hash: "abcd1234", index: 0 },
      value: 100_000_000,
      highest: 100_000_000,
      stats: {
        renewalPeriodStart: 80000,
        renewalPeriodEnd: 110000,
        blocksUntilExpire: 10000,
        daysUntilExpire: 69,
      },
      registered: true,
      expired: false,
    },
    {
      name: "wallet",
      state: "BIDDING",
      height: 99000,
      renewal: null,
      owner: null,
      value: null,
      highest: null,
      stats: {
        bidPeriodStart: 99000,
        bidPeriodEnd: 100000,
        blocksUntilBidding: 0,
        blocksUntilReveal: 1000,
        hoursUntilReveal: 168,
      },
      registered: false,
      expired: false,
    },
  ],

  read_renewals: () => ({
    walletProfileId: "webqa-profile",
    currentHeight: 100_000,
    heightSource: "explorer",
    expiringSoonThresholdDays: 30,
    names: [
      {
        name: "example",
        state: "CLOSED",
        renewalHeight: 100_000 - 105_120 + 10_000,
        expiresAtHeight: 110_000,
        blocksUntilExpire: 10_000,
        daysUntilExpire: 69.4,
        source: "chain",
        expiringSoon: false,
      },
      {
        name: "urgent",
        state: "CLOSED",
        renewalHeight: 100_000 - 105_120 + 1_000,
        expiresAtHeight: 101_000,
        blocksUntilExpire: 1_000,
        daysUntilExpire: 6.9,
        source: "chain",
        expiringSoon: true,
      },
      {
        name: "legacycsv",
        state: "CLOSED",
        renewalHeight: null,
        expiresAtHeight: 500_000,
        blocksUntilExpire: null,
        daysUntilExpire: 42.5,
        source: "csv-import",
        expiringSoon: false,
      },
    ],
  }),

  read_name_info: (_args) => {
    const name = (_args?.name as string) ?? "unknown";
    return {
      name,
      state: "AVAILABLE",
      height: null,
      renewal: null,
      owner: null,
      value: null,
      highest: null,
      stats: null,
      registered: false,
      expired: false,
    };
  },

  // ── Transactions ──────────────────────────────────────────────────────
  read_transactions: () => [
    {
      hash: "deadbeef0001",
      direction: "receive",
      amountDoos: 1_000_000_000,
      amountHns: 10,
      address: "hs1q9g6...(mock)",
      confirmed: true,
      confirmations: 120,
      height: 99880,
      timestamp: new Date(Date.now() - 86_400_000).toISOString(),
      tone: "success",
    },
    {
      hash: "deadbeef0002",
      direction: "send",
      amountDoos: -500_000_000,
      amountHns: -5,
      address: "hs1qxy...(mock)",
      confirmed: true,
      confirmations: 60,
      height: 99940,
      timestamp: new Date(Date.now() - 43_200_000).toISOString(),
      tone: "default",
    },
  ],

  // ── Node ──────────────────────────────────────────────────────────────
  node_status: () => ({
    running: false,
    network: "mainnet",
    height: 100000,
    tip: "000000000000...(mock)",
    peers: 0,
    version: "webqa-mock",
    error: null,
  }),

  start_hsd: () => ({
    running: true,
    network: "mainnet",
    height: 100000,
    tip: "000000000000...(mock)",
    peers: 8,
    version: "webqa-mock",
    message: "Started (mock)",
  }),

  stop_hsd: () => null,
  resync_hsd_chain: () => null,

  // ── Drafts ────────────────────────────────────────────────────────────
  list_tx_drafts: () => [],

  // ── Bid commitment recovery / backup ────────────────────────────────────
  recover_bid_commitment: () => {
    throw new Error("bid value doesn't match any unspent bid coin for this name");
  },
  export_bid_commitments: () => "[]",

  build_send_hns_draft: (_args) => ({
    id: "draft-mock-001",
    walletProfileId: "webqa-profile",
    action: "send_hns",
    status: "draft",
    summary: {
      action: "send_hns",
      sendTotalDoos: (_args?.valueDoos as number) ?? 100_000_000,
      feeDoos: 100_000,
      changeDoos: 0,
      inputTotalDoos: ((_args?.valueDoos as number) ?? 100_000_000) + 100_000,
      numInputs: 1,
      recipientAddress: (_args?.toAddress as string) ?? "hs1q...(mock)",
      txid: null,
      warnings: [],
    },
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  sign_tx_draft: (args) => ({
    id: (args?.draftId as string) ?? "draft-mock-001",
    walletProfileId: "webqa-profile",
    action: "send_hns",
    status: "signed",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  broadcast_tx_draft: (args) => ({
    draftId: (args?.draftId as string) ?? "draft-mock-001",
    txid: "cafebabe0001",
    status: "broadcasted",
  }),

  refresh_tx_confirmations: () => null,

  // ── Name action drafts ────────────────────────────────────────────────
  build_open_draft: () => ({
    id: "draft-open-001",
    walletProfileId: "webqa-profile",
    action: "open",
    status: "draft",
    summary: {
      action: "open",
      sendTotalDoos: 0,
      feeDoos: 100_000,
      changeDoos: 0,
      inputTotalDoos: 100_000,
      numInputs: 1,
      recipientAddress: null,
      txid: null,
      warnings: [],
    },
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_bid_draft: () => ({
    id: "draft-bid-001",
    walletProfileId: "webqa-profile",
    action: "bid",
    status: "draft",
    summary: {
      action: "bid",
      sendTotalDoos: 200_000_000,
      feeDoos: 100_000,
      changeDoos: 0,
      inputTotalDoos: 200_100_000,
      numInputs: 1,
      recipientAddress: null,
      txid: null,
      warnings: [],
    },
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_reveal_draft: () => ({
    id: "draft-reveal-001",
    walletProfileId: "webqa-profile",
    action: "reveal",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_redeem_draft: () => ({
    id: "draft-redeem-001",
    walletProfileId: "webqa-profile",
    action: "redeem",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_register_draft: () => ({
    id: "draft-register-001",
    walletProfileId: "webqa-profile",
    action: "register",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_update_draft: () => ({
    id: "draft-update-001",
    walletProfileId: "webqa-profile",
    action: "update",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_renew_draft: () => ({
    id: "draft-renew-001",
    walletProfileId: "webqa-profile",
    action: "renew",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_transfer_draft: () => ({
    id: "draft-transfer-001",
    walletProfileId: "webqa-profile",
    action: "transfer",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_finalize_draft: () => ({
    id: "draft-finalize-001",
    walletProfileId: "webqa-profile",
    action: "finalize",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_cancel_draft: () => ({
    id: "draft-cancel-001",
    walletProfileId: "webqa-profile",
    action: "cancel",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  build_revoke_draft: () => ({
    id: "draft-revoke-001",
    walletProfileId: "webqa-profile",
    action: "revoke",
    status: "draft",
    summary: null,
    errorMessage: null,
    txid: null,
    confirmationHeight: null,
    createdAt: new Date().toISOString(),
  }),

  // ── DNS resource ──────────────────────────────────────────────────────
  get_resource: () => ({
    records: [{ type: "NS", ns: "ns1.example." }],
  }),

  // ── Assets / Batches (portfolio) ──────────────────────────────────────
  list_assets: () => [],
  get_asset: () => null,
  update_asset: () => null,
  bulk_update_status: () => null,
  bulk_update_tags: () => null,
  delete_asset: () => null,
  import_csv: () => ({ imported: 0, skipped: 0, errors: [] }),
  export_csv: () => 0,

  list_batches: () => [],
  get_batch_with_assets: () => ({ id: 0, name: "", assets: [] }),
  create_batch: () => 1,
  update_batch: () => null,
  delete_batch: () => null,
  add_to_batch: () => null,
  remove_from_batch: () => null,

  // ── Audit / Sync ──────────────────────────────────────────────────────
  get_audit_log: () => [],
  compare_inventory_with_provider: () => ({
    matched: [],
    missing: [],
    extra: [],
  }),

  // ── Namebase (stubs) ──────────────────────────────────────────────────
  get_namebase_status: () => ({ connected: false, has_cookie: false }),
  fetch_namebase_domains: () => ({ domains: [] }),
  fetch_namebase_staked: () => ({ stakedDomains: [] }),
  connect_namebase: () => null,
  disconnect_namebase: () => null,
  import_from_namebase: () => ({ imported: 0, staked_count: 0 }),
  namebase_transfer_domain: () => null,
  fetch_namebase_domain_withdrawals: () => [],
  fetch_namebase_renewals: () => [],
  fetch_namebase_withdrawals: () => [],
  namebase_withdraw_hns: () => null,

  // ── Secure prompt ─────────────────────────────────────────────────────
  secure_prompt_submit: () => null,
  secure_prompt_fetch: () => ({
    promptId: "",
    kind: "info",
    message: "Mock",
  }),
  secure_reveal_backup_phrase: () =>
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
};

/**
 * Execute a mock invoke command.
 * Returns `null` with a console warning for any command not in the map.
 */
export function mockInvoke<T>(cmd: string, args?: Record<string, unknown>): T {
  const handler = handlers[cmd];
  if (handler) {
    return handler(args) as T;
  }
  console.warn(
    `[browser QA] No mock handler for invoke("${cmd}") — returning null.`,
  );
  return null as unknown as T;
}
