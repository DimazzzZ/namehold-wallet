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
