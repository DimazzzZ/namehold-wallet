-- Namebase account-history import: imported custodial-era events.
--
-- Stores events from the Namebase account-history CSV export (or live API).
-- Each row is a Namebase platform event (bid, fee, sale, etc.) with denormalized
-- money columns for efficient querying. The full original JSON is preserved so
-- the UI can display any field we didn't promote to a column.
--
-- This table is separate from on-chain data (assets, transactions) because:
-- 1. It is imported, custodial-era, Namebase-authored data — a different trust
--    domain from on-chain facts.
-- 2. It's append/upsert-by-id, immutable historically. No lifecycle/status.
-- 3. The Activity page joins it in at read time rather than the import writing
--    into the node-derived path.

CREATE TABLE namebase_history (
  id            INTEGER PRIMARY KEY,        -- Namebase event id (natural key, NOT autoincrement)
  created_at    TEXT NOT NULL,              -- ISO-8601 UTC from the export
  type          TEXT NOT NULL,              -- raw event type, e.g. 'auctions:place-bid:4'
  family        TEXT NOT NULL,              -- event family, e.g. 'auctions' | 'subdomains' | 'marketplace' | ...
  verb          TEXT NOT NULL,              -- event verb, e.g. 'place-bid' | 'confirm-transfer' | ...
  name          TEXT,                       -- normalized domainName/domain (lowercased, no dot), NULL for non-name events
  -- Denormalized, parsed money columns for cheap querying/aggregation.
  -- All *_doos are HNS base units (i64); usd_cents is USD base units (i64).
  fee_doos      INTEGER,                    -- feeChargedString / prepaidFeeString / hnsFeeAmountString / totalFeeString, whichever applies
  bid_doos      INTEGER,                    -- bidAmountString
  stake_doos    INTEGER,                    -- stakeAmountString
  usd_cents     INTEGER,                    -- deliveredAmountUsd / receivedAmount (sale proceeds)
  hns_doos      INTEGER,                    -- deliveredAmountHns / deposit amount
  -- Stable Namebase correlation IDs (any may be NULL depending on event type).
  auction_id    TEXT,
  bid_id        TEXT,
  sale_id       TEXT,
  -- Full original JSON so the UI/detail view can show anything we didn't promote
  -- to a column, and so re-classification never needs a re-import.
  data_json     TEXT NOT NULL,
  imported_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_nb_history_name    ON namebase_history(name);
CREATE INDEX idx_nb_history_type    ON namebase_history(type);
CREATE INDEX idx_nb_history_created ON namebase_history(created_at);
