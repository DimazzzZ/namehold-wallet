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
  /**
   * The name's on-chain auction state: AVAILABLE, OPENING, BIDDING, REVEAL,
   * CLOSED, TRANSFER, REVOKED, etc.  The backend synthesizes `"AVAILABLE"`
   * for names that have never been opened (node confirms the name is valid
   * but `getnameinfo.info` is null, or the explorer returns 404).
   */
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
  /** True when the name is registered (CLOSED + owned). */
  registered?: boolean | null;
  /** True when the name's registration has expired. */
  expired?: boolean | null;
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

/**
 * Full DNS resource object from `getnameresource` (returned by
 * `read_name_records`). Contains the records array plus resource-level
 * metadata (TTL, serial). The backend guarantees `records` is always present
 * as an array (never null/missing), even on degrade paths.
 */
export interface NameResource {
  records: Record<string, unknown>[];
  /** Resource-level TTL (seconds), if set by the name owner. */
  ttl?: number | null;
  /** Resource serial number. */
  serial?: number | null;
  /** Any additional fields hsd may include in future versions. */
  [key: string]: unknown;
}

/**
 * Compact block details from `read_block_info` (node-only). The backend
 * soft-degrades to `null` when no synced node is reachable, so the frontend
 * must handle a nullable query result.
 */
export interface BlockInfo {
  height: number;
  /** Block hash (display-order hex). */
  hash: string;
  /** Unix timestamp (seconds since epoch). */
  time: number;
  /** Number of transactions in the block. */
  txCount: number;
  /** Miner reward in doos (coinbase output sum = subsidy + fees). */
  minerReward: number;
  /** Block difficulty. */
  difficulty: number;
}

/**
 * Compact transaction details from `read_tx_info` (node-only). The backend
 * soft-degrades to `null` when no synced node is reachable or the tx is
 * unknown, so the frontend must handle a nullable query result. All amounts
 * are in doos (backend converts hsd's HNS floats to doos so the frontend
 * amount contract stays uniform).
 */
export interface TxInfo {
  txid: string;
  confirmations: number;
  /** Block height, or -1 when unconfirmed. */
  height: number;
  /** Confirming block hash, or null when unconfirmed. */
  block: string | null;
  /** Unix timestamp (seconds); 0 when unconfirmed. */
  time: number;
  /**
   * Fee in doos, or `null` when the node couldn't determine it (coinbase
   * transactions, or an hsd response with unresolved input coins). The modal
   * renders `null` as `—`; a real `0` fee wouldn't happen on a normal HNS
   * tx, so a displayed `0` was always the misleading case.
   */
  fee: number | null;
  inputsCount: number;
  outputsCount: number;
  /** Sum of output values in doos. */
  totalOut: number;
}

/**
 * Discriminated-error result `read_tx_info` returns when the node can respond
 * but is missing a capability required to fill the response shape. Currently
 * only `tx_index_disabled` (the node lacks `--index-tx`). Kept as a distinct
 * shape so the modal can show a targeted hint instead of the generic
 * "requires synced node" message.
 */
export interface TxInfoError {
  error: "tx_index_disabled";
}

/** Narrows a `read_tx_info` result to the error shape. */
export function isTxInfoError(
  v: TxInfo | TxInfoError | null | undefined,
): v is TxInfoError {
  return v != null && typeof v === "object" && "error" in v;
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
  /**
   * "true" | "false" — start hsd automatically when the app launches. Default
   * "true". Honored only at app-launch time (a runtime toggle affects the next
   * launch, not the current process).
   */
  autostart_hsd: string;
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
  /**
   * "true" | "false" — OS notifications for reveal/renewal deadlines (I1).
   * Opt-in: off by default so no OS permission prompt fires unasked.
   */
  deadline_notify_enabled: string;
  /** Integer string, blocks of lead time before a reveal window closes. */
  deadline_notify_reveal_lead_blocks: string;
  /** Float string, days of lead time before a renewal is due. */
  deadline_notify_renewal_lead_days: string;
  /**
   * "1" | "0" — run the background sync daemon (`namehold-syncd`) so wallet
   * state keeps updating even when the app is closed. Default "1" (on). When
   * enabled, hsd is left running on app exit instead of being killed, so the
   * daemon has a node to talk to.
   */
  background_sync_enabled: string;
  /**
   * "full" | "spv" — hsd node operating mode. Default "full".
   * - "full": full node with --index-address --index-tx (current behavior)
   * - "spv": SPV mode with --spv (faster sync, less disk, explorer-dependent)
   * Only relevant when chain_source is "local_node" or "remote_node".
   */
  node_mode: string;
  /** Fallback explorer URL used when primary explorer_api_url is unreachable. */
  explorer_fallback_url: string;
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
  /** Stamped once at the end of a clean explorer-driven sync run (repair +
   *  discover), separately from `lastSyncedAt` (which only the node-RPC
   *  step advances). In explorer-only mode (no local node) this is the
   *  only freshness signal that ever moves. */
  lastExplorerSyncAt: string | null;
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
  /** The name a name-covenant action (open/bid/reveal/…) applies to. Written
   *  by the backend for name-action drafts; absent for plain sends. */
  name?: string | null;
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

/**
 * Result of `sign_name_message`: an hsd `signmessagewithname`-compatible
 * signature over an exact message, produced with the wallet key that owns a
 * name — used for third-party domain-claim verification (e.g. Namebase).
 * Not a spend; the private key never leaves the backend.
 */
export interface NameSignature {
  /** base64 of a 64-byte compact (low-S, non-recoverable) ECDSA signature. */
  signature: string;
  /** hex-encoded 33-byte compressed public key of the owning key. */
  publicKey: string;
  /** The owner address the key derives to, for cross-checking. */
  address: string;
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
  | "auctions"
  | "activity"
  | "settings";

export type PortfolioSectionKey = "inventory" | "batches" | "renewals" | "dns";

export type MigrationSectionKey = "namebase" | "sync";

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

// ---------------------------------------------------------------------------
// Auction capability / task-state types
// ---------------------------------------------------------------------------

export type AuctionTaskState =
  | "availableToOpen"
  | "waitingForBidding"
  | "readyToBid"
  | "readyToReveal"
  | "revealBroadcastPending"
  | "revealDoneWaitingForClose"
  | "wonNeedsRegister"
  | "lostNeedsRedeem"
  | "transferPendingFinalize"
  | "ownedNoUrgentAction"
  | "expiringSoon"
  | "unavailableOther";

export interface NameActionCapability {
  allowed: boolean;
  reason: string | null;
}

export interface NameActionCapabilities {
  name: string;
  phase: string;
  taskState: AuctionTaskState;
  ownsName: boolean;
  hasBidCommitment: boolean;
  /** Unspent COV_BID coin for this name — what a REVEAL actually spends.
   * Gates `canReveal`. Backend fix (Task 6 / I2 Part 3): `hasRevealCoin`
   * (below) only ever becomes true AFTER a reveal, so it can never gate
   * revealing itself — it gates `canRedeem` instead. */
  hasBidCoin: boolean;
  hasRevealCoin: boolean;
  hasOwnerCoin: boolean;
  /** The txid of the wallet's reveal broadcast, if any. Feeds the reveal
   * card's explorer link + copy button. Null until a reveal is broadcast
   * (or observed on-chain by chain scan). */
  revealTxid: string | null;
  /** The wallet's true bid value (doos) from the local commitment row, so the
   * confirm-before-broadcast panel can show the amount. Null when unknown. */
  bidValueDoos: number | null;
  canOpen: NameActionCapability;
  canBid: NameActionCapability;
  canReveal: NameActionCapability;
  canRedeem: NameActionCapability;
  canRegister: NameActionCapability;
  canUpdate: NameActionCapability;
  canTransfer: NameActionCapability;
  canFinalize: NameActionCapability;
  canCancelTransfer: NameActionCapability;
  canRenew: NameActionCapability;
  canRevoke: NameActionCapability;
  nextActionKey: string | null;
  nextActionLabel: string | null;
  nextActionReason: string | null;
  countdownLabel: string | null;
  countdownBlocks: number | null;
  countdownHours: number | null;
}

// ---------------------------------------------------------------------------
// Name bids (explorer-backed per-bid detail, Task 1 / Task 2)
// ---------------------------------------------------------------------------

export interface NameBid {
  txid: string | null;
  index: number | null;
  /** The public LOCKUP (doos) — an upper bound, NOT the true bid. */
  lockup: number | null;
  /** The revealed true bid (doos); null/absent pre-REVEAL for others. */
  value: number | null;
  /** Whether THIS bid has been revealed on-chain. */
  revealed: boolean | null;
  /** Winner flag (meaningful at CLOSED). */
  win: boolean | null;
  reveal: unknown | null;
  time: number | null;
  /** True iff this bid matches one of MY local bid_commitments. */
  mine: boolean;
  /** MY plaintext true bid (doos) — present only when `mine` is true. */
  myValue: number | null;
}

export interface NameBids {
  name: string;
  state: string | null;
  /** Top-level high bid (doos), populated at REVEAL/CLOSED. */
  highest: number | null;
  value: number | null;
  bids: NameBid[];
  myBidCount: number;
}

// ---------------------------------------------------------------------------
// Renewals (chain-driven, Task 3 / C3)
// ---------------------------------------------------------------------------

/** Where a renewal row's expiry data comes from. */
export type RenewalSource = "chain" | "csv-import";

/** Where the current chain height used for the computation comes from. */
export type HeightSource = "node" | "explorer" | "unknown";

/** One name in the Renewals view (`read_renewals`). */
export interface RenewalRow {
  name: string;
  state: string | null;
  renewalHeight: number | null;
  expiresAtHeight: number | null;
  blocksUntilExpire: number | null;
  daysUntilExpire: number | null;
  source: RenewalSource;
  expiringSoon: boolean;
}

/** Response of `read_renewals`. */
export interface RenewalsResponse {
  walletProfileId: string | null;
  currentHeight: number | null;
  heightSource: HeightSource;
  expiringSoonThresholdDays: number;
  names: RenewalRow[];
}

/** Result of a successful `recover_bid_commitment` call. Non-secret. */
export interface RecoveredBidCommitment {
  name: string;
  address: string;
  bidValueDoos: number;
  lockupValueDoos: number;
}
