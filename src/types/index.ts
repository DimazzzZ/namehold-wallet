export type MigrationStatus =
  | "not_started"
  | "namebase_transfer_requested"
  | "waiting_transfer_tx"
  | "transfer_seen_on_chain"
  | "waiting_finalize"
  | "finalized_owned"
  | "failed_or_stuck"
  | "do_not_touch_staked";

export interface Asset {
  id: number;
  tld: string;
  status: MigrationStatus;
  is_staked: boolean;
  category: string | null;
  tags: string[];
  notes: string | null;
  hns_received: number | null;
  transfer_tx_hash: string | null;
  finalize_tx_hash: string | null;
  name_state: string | null;
  expires_at_height: number | null;
  days_until_expire: number | null;
  last_synced_at: string | null;
  created_at: string;
  updated_at: string;
}

export type BatchStatus =
  | "planned"
  | "in_progress"
  | "completed"
  | "paused"
  | "cancelled";

export interface Batch {
  id: number;
  name: string;
  description: string | null;
  status: BatchStatus;
  asset_count: number | null;
  created_at: string;
  updated_at: string;
}

export interface BatchWithAssets extends Batch {
  assets: Asset[];
}

export interface HsdBalance {
  confirmed: number;
  unconfirmed: number;
  locked_unconfirmed: number | null;
  locked_confirmed: number | null;
}

export interface HsdName {
  name: string;
  state: string | null;
  height: number | null;
  renewal: number | null;
  owner: { hash: string; index: number } | null;
  value?: number | null;
  highest?: number | null;
  /**
   * Auction state stats from hsd `getnameinfo` (camelCase, as the Rust `HsdName`
   * serializes). All optional: only the fields for the name's current phase are
   * present, and the explorer path may omit the auction ones entirely.
   */
  stats: HsdNameStats | null;
  /** Non-zero block height while the name is mid-transfer (0/null otherwise). */
  transfer?: number | null;
}

export interface HsdNameStats {
  renewalPeriodStart?: number | null;
  renewalPeriodEnd?: number | null;
  blocksUntilExpire?: number | null;
  daysUntilExpire?: number | null;
  // Auction phase windows + countdowns (present only in the relevant phase).
  openPeriodStart?: number | null;
  openPeriodEnd?: number | null;
  bidPeriodStart?: number | null;
  bidPeriodEnd?: number | null;
  revealPeriodStart?: number | null;
  revealPeriodEnd?: number | null;
  blocksUntilOpen?: number | null;
  blocksUntilBidding?: number | null;
  blocksUntilReveal?: number | null;
  blocksUntilClose?: number | null;
  hoursUntilOpen?: number | null;
  hoursUntilBidding?: number | null;
  hoursUntilReveal?: number | null;
  hoursUntilClose?: number | null;
}

export interface WalletConnection {
  connected: boolean;
  info?: unknown;
  error?: string;
}

export interface DashboardStats {
  total: number;
  staked: number;
  unstaked: number;
  status_counts: Record<string, number>;
  recent_audit: AuditEntry[];
}

export interface AuditEntry {
  id: number;
  timestamp: string;
  action: string;
  entity: string | null;
  entity_id: number | null;
  detail: string | null;
  created_at: string;
}

export interface ImportResult {
  imported: number;
  skipped: number;
  errors: string[];
}

export interface SyncResult {
  matched: number;
  wallet_count: number;
  extra_count: number;
  extra_names: string[];
  missing_count: number;
  missing_names: string[];
  errors: string[];
}

export interface SyncReport {
  matched: string[];
  missing: string[];
  extra: string[];
}

export interface WalletSnapshot {
  id: number;
  snapshot_at: string;
  wallet_name: string;
  balance: number;
  address: string | null;
  name_count: number;
}

/**
 * The single non-custodial settings model. Reads come from the explorer
 * (`explorer_api_url`); sending uses one hsd node (`node_rpc_url`). Keys are
 * local; there is no legacy hsd-wallet / connection-mode config.
 */
export interface Settings {
  /** hsd node RPC used for sync + broadcast (sending). */
  node_rpc_url: string;
  node_rpc_api_key: string;
  /** hsd data directory ("prefix") used when the app starts hsd. Empty = ~/.hsd. */
  hsd_prefix: string;
  /** Explicit path to the hsd binary. Empty = auto-discover (common dirs + PATH). */
  hsd_path: string;
  /** HNSFans explorer used for node-free reads (balance + names). */
  explorer_api_url: string;
  /** Integer string, default "20". */
  address_gap_limit: string;
  /** Integer string seconds, default "900". */
  signer_session_timeout_seconds: string;
  /** "true" | "false" — reveals advanced nav items and settings sections. */
  advanced_mode: string;
  /** "true" | "false" — marks first-run onboarding as complete. */
  onboarding_complete: string;
}

// ---------------------------------------------------------------------------
// Non-custodial wallet types (secret-free; mirror src-tauri noncustodial::types)
// ---------------------------------------------------------------------------

export type WalletNetwork = "mainnet" | "testnet" | "regtest";
export type WalletProfileKind = "mnemonic_hot" | "xpriv_hot" | "watch_only_xpub";

export interface WalletProfileSummary {
  id: string;
  label: string;
  kind: WalletProfileKind;
  network: WalletNetwork;
  accountXpub: string;
  accountIndex: number;
  receiveDepth: number;
  changeDepth: number;
  receiveAddress: string | null;
  lastSyncedHeight: number | null;
  lastSyncedAt: string | null;
  watchOnly: boolean;
  /** False when the wallet was created without a passphrase (kdf='none'); the
   *  signer then unlocks in one click with no passphrase prompt. */
  hasPassphrase: boolean;
  active: boolean;
}

export interface SignerSessionSummary {
  walletProfileId: string | null;
  unlocked: boolean;
  unlockedUntilEpochMs: number;
}

export interface TxSummary {
  action: string;
  sendTotalDoos: number;
  feeDoos: number;
  changeDoos: number;
  inputTotalDoos: number;
  numInputs: number;
  recipientAddress: string | null;
  txid: string | null;
  warnings: string[];
}

export interface TxDraftSummary {
  id: string;
  walletProfileId: string;
  action: string;
  status:
    | "draft"
    | "signed"
    | "broadcast_pending"
    | "broadcasted"
    | "confirmed"
    | "dropped"
    | "failed";
  summary: TxSummary | null;
  errorMessage: string | null;
  txid: string | null;
  /** Block height the tx was mined at, once `status` is "confirmed". */
  confirmationHeight: number | null;
  createdAt: string;
}

export interface BroadcastResult {
  draftId: string;
  txid: string;
  status: string;
}

export interface WriteCapability {
  signerUnlocked: boolean;
  broadcasterAvailable: boolean;
  canWrite: boolean;
  reason: string | null;
}

export interface WalletBalances {
  liquidDoos: number;
  nameControlDoos: number;
  nameLockupDoos: number;
  totalDoos: number;
}

// ---------------------------------------------------------------------------
// Read model (explorer-backed, node-free)
// ---------------------------------------------------------------------------

export interface WalletReadModel {
  address: string | null;
  watchAddresses: string[];
  balance: HsdBalance | null;
  names: HsdName[];
  transactions: WalletTransactionRow[];
  lastUpdatedAt?: string | null;
}

// ---------------------------------------------------------------------------
// Frontend UI-facing types (routing, shell status, workspace tabs, view models)
// ---------------------------------------------------------------------------

export type AppRouteKey =
  | "portfolio"
  | "migration"
  | "wallet"
  | "settings";

export type PortfolioSectionKey = "inventory" | "batches" | "renewals" | "dns";

export type MigrationSectionKey = "namebase" | "transfers" | "sync";

export type StatusTone = "default" | "info" | "success" | "warning" | "error";

export interface ShellStatusItem {
  key: string;
  label: string;
  value: string;
  tone: StatusTone;
  detail?: string;
  route?: string;
}

export interface PageAction {
  label: string;
  variant?: "primary" | "secondary" | "danger" | "ghost";
  disabled?: boolean;
  loading?: boolean;
  to?: string;
  onClick?: () => void;
}

export interface WorkspaceTab<T extends string> {
  key: T;
  label: string;
  description?: string;
  badge?: string | number;
}

export interface WalletTransactionRow {
  hash: string;
  direction: "send" | "receive" | "other";
  amountDoos: number;
  amountHns: number;
  address: string;
  confirmed: boolean;
  confirmations: number | null;
  height: number | null;
  timestamp: string | null;
  tone: StatusTone;
}

