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
}
