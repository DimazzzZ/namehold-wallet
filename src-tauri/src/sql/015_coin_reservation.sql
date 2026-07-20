-- Reserve tracked UTXOs for the draft that selected them, so two drafts built
-- before either broadcasts cannot pick the same coins (I3). A NULL value
-- means the coin is free; a non-NULL value names the `wallet_tx_drafts.id`
-- that currently claims it.
--
-- There is deliberately no separate "reserved_at" timestamp: the claiming
-- draft's own `created_at` (in `wallet_tx_drafts`) doubles as the reservation
-- age for TTL purposes (see `noncustodial::send::RESERVATION_TTL_SECS`), so a
-- stale reservation left behind by an abandoned build can be recognized and
-- opportunistically cleared without an extra column.
ALTER TABLE tracked_utxos ADD COLUMN reserved_by_draft_id TEXT;

CREATE INDEX IF NOT EXISTS idx_utxo_reserved_by_draft ON tracked_utxos(reserved_by_draft_id);
