import { z } from "zod";

export const MigrationStatus = z.enum([
  "not_started",
  "namebase_transfer_requested",
  "waiting_transfer_tx",
  "transfer_seen_on_chain",
  "waiting_finalize",
  "finalized_owned",
  "failed_or_stuck",
  "do_not_touch_staked",
]);

export const BatchStatus = z.enum([
  "planned",
  "in_progress",
  "completed",
  "paused",
  "cancelled",
]);

export const AssetSchema = z.object({
  id: z.number(),
  tld: z.string(),
  status: MigrationStatus,
  is_staked: z.boolean(),
  category: z.string().nullable(),
  tags: z.array(z.string()),
  notes: z.string().nullable(),
  hns_received: z.number().nullable(),
  transfer_tx_hash: z.string().nullable(),
  finalize_tx_hash: z.string().nullable(),
  name_state: z.string().nullable(),
  expires_at_height: z.number().nullable(),
  days_until_expire: z.number().nullable(),
  last_synced_at: z.string().nullable(),
  created_at: z.string(),
  updated_at: z.string(),
});

export const BatchSchema = z.object({
  id: z.number(),
  name: z.string(),
  description: z.string().nullable(),
  status: BatchStatus,
  asset_count: z.number().nullable(),
  created_at: z.string(),
  updated_at: z.string(),
});

export const HsdBalanceSchema = z.object({
  confirmed: z.number(),
  unconfirmed: z.number(),
  locked_unconfirmed: z.number().nullable(),
  locked_confirmed: z.number().nullable(),
});

export const ActionRowSchema = z.object({
  txid: z.string(),
  action: z.string(),
  name: z.string().nullable(),
  nameHash: z.string().nullable(),
  valueDoos: z.number(),
  direction: z.string(),
  height: z.number().nullable(),
  time: z.number().nullable(),
  confirmed: z.boolean(),
  counterparty: z.string().nullable(),
});

export type ActionRow = z.infer<typeof ActionRowSchema>;

export const NamebaseHistoryRowSchema = z.object({
  id: z.number(),
  createdAt: z.string(),
  type: z.string(),
  family: z.string(),
  verb: z.string(),
  name: z.string().nullable(),
  feeDoos: z.number().nullable(),
  bidDoos: z.number().nullable(),
  stakeDoos: z.number().nullable(),
  usdCents: z.number().nullable(),
  hnsDoos: z.number().nullable(),
  auctionId: z.string().nullable(),
  bidId: z.string().nullable(),
  saleId: z.string().nullable(),
  dataJson: z.string(),
  importedAt: z.string(),
});

export type NamebaseHistoryRow = z.infer<typeof NamebaseHistoryRowSchema>;
