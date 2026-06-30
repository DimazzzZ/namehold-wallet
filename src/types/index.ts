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
  stats: {
    renewal_period_end: number | null;
    blocks_until_expire: number | null;
    days_until_expire: number | null;
  } | null;
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

export interface Settings {
  hsd_wallet_api_url: string;
  hsd_node_api_url: string;
  hsd_api_key: string;
  hsd_wallet_id: string;
  hsd_network: string;
  hsd_prefix: string;
  write_mode: string;
  connection_mode:
    | "local_managed_hsd"
    | "remote_hsd"
    | "auto_fallback"
    | "external_read_only";
  external_read_provider: "none" | "hnsfans";
  external_read_api_url: string;
  /** JSON stringified string[] of watch addresses. */
  external_read_watch_addresses: string;
  /** JSON stringified string[] of watch names. */
  external_read_watch_names: string;
  remote_hsd_label: string;
  /** "true" | "false" */
  trusted_remote_hsd: string;
  future_signer_mode: "none" | "local_signer_planned";
  /** "true" | "false" — reveals advanced nav items and settings sections. */
  advanced_mode: string;
  /** "true" | "false" — marks first-run onboarding as complete. */
  onboarding_complete: string;
}

// ---------------------------------------------------------------------------
// Provider / connection-mode capability types
// ---------------------------------------------------------------------------

export type ConnectionMode = Settings["connection_mode"];
export type ReadProviderKind = "local_hsd" | "remote_hsd" | "external_hnsfans";
export type WriteProviderKind = "local_hsd" | "remote_hsd" | "none";

export interface ProviderStatus {
  kind: ReadProviderKind;
  label: string;
  healthy: boolean;
  writeCapable: boolean;
  /** Whether NodeControl may start/stop the backend. */
  manageable: boolean;
  /** Fallback or failure explanation. */
  reason?: string;
  providerUrl?: string | null;
  network?: string | null;
  chainHeight?: number | null;
  verificationProgress?: number | null;
  syncing?: boolean;
}

export interface ReadContext {
  connectionMode: ConnectionMode;
  activeReadProvider: ProviderStatus;
  fallbackActive: boolean;
  localNodeHealthy: boolean;
  walletAvailable: boolean;
  writeAllowed: boolean;
  writeReason?: string | null;
}

export interface WalletReadModel {
  context: ReadContext;
  address: string | null;
  watchAddresses: string[];
  balance: HsdBalance | null;
  names: HsdName[];
  transactions: WalletTransactionRow[];
  lastUpdatedAt?: string | null;
  readOnlyReason?: string | null;
}

export interface ExternalNameSummary {
  name: string;
  state: string | null;
  ownerAddress?: string | null;
  expiresAtHeight?: number | null;
  daysUntilExpire?: number | null;
  source: "external_hnsfans";
}

export interface ExternalTransactionRow extends WalletTransactionRow {
  source: "external_hnsfans";
  matchedAddress?: string | null;
}

// ---------------------------------------------------------------------------
// Frontend UI-facing types (routing, shell status, workspace tabs, view models)
// ---------------------------------------------------------------------------

export type AppRouteKey =
  | "overview"
  | "portfolio"
  | "migration"
  | "wallet"
  | "node"
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

export interface OverviewMetric {
  key: string;
  label: string;
  value: string | number;
  hint?: string;
  tone?: StatusTone;
}

export interface OverviewData {
  metrics: OverviewMetric[];
  statusCounts: Record<string, number>;
  recentAudit: AuditEntry[];
  namebaseConnected: boolean;
  namebaseHnsBalance?: number;
  readContext?: ReadContext | null;
  walletSummary?: WalletReadModel | null;
  providerWarnings?: string[];
}
