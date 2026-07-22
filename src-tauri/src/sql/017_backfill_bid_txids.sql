-- One-shot backfill for bid_commitments.bid_txid / reveal_txid.
--
-- Before the fix that made build_bid_draft / build_reveal_draft persist the
-- broadcast txid onto the commitment as it's built, bid/reveal txids were
-- never written back onto bid_commitments — so pre-existing commitments
-- (placed before that fix landed) show bid_txid/reveal_txid = NULL forever,
-- and the "which bids are mine" panel can't recognize them.
--
-- This backfills those NULLs from the matching wallet_tx_drafts row for the
-- same (wallet_profile_id, name). The link is unique because of the
-- "one bid per wallet per name" rule enforced elsewhere.
--
-- The txid is sourced from json_extract(summary_json,'$.txid') — the
-- deterministic on-chain txid computed at BUILD time (before the witness is
-- attached, which is why the top-level wallet_tx_drafts.txid column — only
-- populated on broadcast — historically lagged behind). Falls back to the
-- top-level column via COALESCE for robustness. Written verbatim: Handshake
-- txids/outpoints are NOT byte-reversed like Bitcoin, so there is no
-- reversal here.
--
-- Idempotent and non-destructive: each UPDATE is gated on the target column
-- already being NULL, so an already-backfilled (or freshly-built, per the
-- forward fix) row is never touched, and re-running this migration is a
-- no-op. Only non-terminal-good draft statuses are eligible; 'draft',
-- 'dropped', and 'failed' drafts are excluded. When multiple qualifying
-- drafts exist for the same (profile, name) — e.g. a dropped duplicate that
-- got retried — the best status wins: confirmed > broadcasted >
-- broadcast_pending > signed.

UPDATE bid_commitments SET bid_txid = (
    SELECT COALESCE(json_extract(d.summary_json, '$.txid'), d.txid)
    FROM wallet_tx_drafts d
    WHERE d.wallet_profile_id = bid_commitments.wallet_profile_id
      AND json_extract(d.summary_json, '$.name') = bid_commitments.name
      AND d.action = 'bid'
      AND d.status IN ('confirmed', 'broadcasted', 'broadcast_pending', 'signed')
      AND COALESCE(json_extract(d.summary_json, '$.txid'), d.txid) IS NOT NULL
    ORDER BY CASE d.status
        WHEN 'confirmed' THEN 0
        WHEN 'broadcasted' THEN 1
        WHEN 'broadcast_pending' THEN 2
        ELSE 3
    END
    LIMIT 1
)
WHERE bid_txid IS NULL
  AND EXISTS (
    SELECT 1 FROM wallet_tx_drafts d
    WHERE d.wallet_profile_id = bid_commitments.wallet_profile_id
      AND json_extract(d.summary_json, '$.name') = bid_commitments.name
      AND d.action = 'bid'
      AND d.status IN ('confirmed', 'broadcasted', 'broadcast_pending', 'signed')
      AND COALESCE(json_extract(d.summary_json, '$.txid'), d.txid) IS NOT NULL
  );

UPDATE bid_commitments SET reveal_txid = (
    SELECT COALESCE(json_extract(d.summary_json, '$.txid'), d.txid)
    FROM wallet_tx_drafts d
    WHERE d.wallet_profile_id = bid_commitments.wallet_profile_id
      AND json_extract(d.summary_json, '$.name') = bid_commitments.name
      AND d.action = 'reveal'
      AND d.status IN ('confirmed', 'broadcasted', 'broadcast_pending', 'signed')
      AND COALESCE(json_extract(d.summary_json, '$.txid'), d.txid) IS NOT NULL
    ORDER BY CASE d.status
        WHEN 'confirmed' THEN 0
        WHEN 'broadcasted' THEN 1
        WHEN 'broadcast_pending' THEN 2
        ELSE 3
    END
    LIMIT 1
)
WHERE reveal_txid IS NULL
  AND EXISTS (
    SELECT 1 FROM wallet_tx_drafts d
    WHERE d.wallet_profile_id = bid_commitments.wallet_profile_id
      AND json_extract(d.summary_json, '$.name') = bid_commitments.name
      AND d.action = 'reveal'
      AND d.status IN ('confirmed', 'broadcasted', 'broadcast_pending', 'signed')
      AND COALESCE(json_extract(d.summary_json, '$.txid'), d.txid) IS NOT NULL
  );
