-- Paid swap offers: seller-side tracking for atomic finalizeWithPayment swaps.
-- When a seller initiates a "sell with payment" flow, an offer is recorded here.
-- The buyer then builds a finalize-with-payment tx; once confirmed, the seller
-- can "claim" the payment by verifying the tx contains a P2WPKH output ≥ price.
CREATE TABLE IF NOT EXISTS paid_swap_offers (
    name TEXT PRIMARY KEY,
    buyer_address TEXT NOT NULL,
    price_doos INTEGER NOT NULL,
    transfer_txid TEXT,
    claimed INTEGER DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Index for listing pending offers (transfer_txid IS NULL and claimed = 0).
CREATE INDEX IF NOT EXISTS idx_paid_swap_offers_pending
    ON paid_swap_offers(claimed, transfer_txid)
    WHERE transfer_txid IS NULL AND claimed = 0;
