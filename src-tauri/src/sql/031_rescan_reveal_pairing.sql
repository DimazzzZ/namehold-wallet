-- Re-scan: reveal values recorded before this migration may sit on the wrong
-- bid.
--
-- The scanner used to attach a REVEAL to "the earliest bid for this name not
-- yet matched". That is indistinguishable from the truth while a wallet holds
-- one bid per name, and meaningless once one transaction reveals several: the
-- disclosed amounts land on whichever rows the ordering happened to pick. On a
-- real auction with four own bids, every one of the four was wrong.
--
-- Handshake pairs a name covenant with the coin spent at the same index
-- (`rules.verifyCovenants`), so the correct pairing was always recoverable from
-- the block — the scanner just was not reading it. It does now, but only for
-- blocks it walks from here on, and its cursor has long passed the ones that
-- matter.
--
-- Nothing here is derived data the user would lose: the index is a cache of the
-- chain, and the wallet's own bids live in `bid_commitments`, untouched.

DELETE FROM name_bid_outpoints;
UPDATE chain_scan_cursor SET last_height = 0;
