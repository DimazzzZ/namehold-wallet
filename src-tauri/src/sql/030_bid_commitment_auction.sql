-- Record which auction a bid commitment belongs to.
--
-- `bid_commitments` was keyed by name, and a name can be auctioned many times:
-- an auction nobody reveals in lapses and the name becomes available again. So
-- a commitment from a dead auction kept counting as one of the wallet's bids on
-- the live one — the name modal's header read "Latest bid … · 2 of yours" while
-- the bids panel right below it, which scopes by auction (029), said "yours: 1".
--
-- Existing rows are backfilled from `reveal_end_height`, which `build_bid_draft`
-- derived from the very height we want:
--
--   reveal_end = start + (tree_interval + 1) + bidding_period + reveal_period
--
-- so the offset is a per-network constant (`Network::name_params`, mirroring
-- hsd): 37 + 720 + 1440 = 2197 on mainnet, 37 + 144 + 288 = 469 on testnet,
-- 6 + 5 + 10 = 21 on regtest. `wallet_profiles.network` cannot be 'simnet'
-- (its CHECK excludes it), so those three cover every row.
--
-- A commitment with no `reveal_end_height` — one recovered from the chain
-- rather than built here, see 014 — keeps a NULL start height. The read side
-- treats "unknown auction" as "could be this one" rather than hiding a bid the
-- wallet may still need to reveal.

ALTER TABLE bid_commitments ADD COLUMN name_start_height INTEGER;

UPDATE bid_commitments
   SET name_start_height = reveal_end_height - (
         SELECT CASE p.network
                  WHEN 'mainnet' THEN 2197
                  WHEN 'testnet' THEN 469
                  WHEN 'regtest' THEN 21
                END
           FROM wallet_profiles p
          WHERE p.id = bid_commitments.wallet_profile_id
       )
 WHERE reveal_end_height IS NOT NULL;
