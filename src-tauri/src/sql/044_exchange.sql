-- HNS <-> BTC exchange (docs/specs/2026-11-07-hns-btc-exchange.md).

-- One per profile that enabled the exchange: the public keys the daemon
-- derives from without the phrase, and the next indexes to hand out.
CREATE TABLE IF NOT EXISTS exchange_accounts (
    profile_id        TEXT PRIMARY KEY,
    network           TEXT NOT NULL,
    -- 33-byte public key || 32-byte chain code, hex, of m/84'/c'/0'.
    btc_xpub_hex      TEXT NOT NULL,
    -- The same of m/44'/<coin type>'/<account>'/3'.
    swap_xpub_hex     TEXT NOT NULL,
    account           INTEGER NOT NULL,
    next_trade_index  INTEGER NOT NULL DEFAULT 0,
    next_btc_receive  INTEGER NOT NULL DEFAULT 0,
    next_btc_change   INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL
);

-- This profile's own offers.
CREATE TABLE IF NOT EXISTS exchange_offers (
    id                 TEXT PRIMARY KEY,
    profile_id         TEXT NOT NULL,
    network            TEXT NOT NULL,
    side               TEXT NOT NULL CHECK (side IN ('sell', 'buy')),
    signed_json        TEXT NOT NULL,
    -- The offer's relay key (signs messages only; controls no coin).
    relay_key_hex      TEXT NOT NULL,
    state              TEXT NOT NULL CHECK (state IN ('live', 'withdrawn')),
    created_at         INTEGER NOT NULL,
    last_published_at  INTEGER,
    last_error         TEXT
);
CREATE INDEX IF NOT EXISTS exchange_offers_profile ON exchange_offers (profile_id);

-- A Buyer-maker's hashes, computed at the offer's creation from the
-- phrase, so the daemon can answer a take without it.
CREATE TABLE IF NOT EXISTS exchange_hash_pool (
    profile_id   TEXT NOT NULL,
    trade_index  INTEGER NOT NULL,
    hash_hex     TEXT NOT NULL,
    -- The secret's nonce (noncustodial/exchange/keys.rs).
    nonce_hex    TEXT NOT NULL,
    used         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_id, trade_index)
);

CREATE TABLE IF NOT EXISTS exchange_trades (
    profile_id         TEXT NOT NULL,
    trade_id           TEXT NOT NULL,
    network            TEXT NOT NULL,
    role               TEXT NOT NULL CHECK (role IN ('buyer', 'seller')),
    maker              INTEGER NOT NULL,
    offer_id           TEXT NOT NULL,
    offer_json         TEXT NOT NULL,
    trade_index        INTEGER NOT NULL,
    relay_key_hex      TEXT NOT NULL,
    counterparty_key   TEXT,
    hns_amount         INTEGER NOT NULL,
    btc_sats           INTEGER NOT NULL,
    price_msat         INTEGER NOT NULL,
    terms_json         TEXT,
    terms_id           TEXT,
    -- engine::Holdings as JSON. Holds the Buyer's secret from its Commit
    -- until the trade ends (spec §6, "The Buyer's secret on disk").
    holdings_json      TEXT,
    status_json        TEXT,
    declined           TEXT,
    last_error         TEXT,
    watchtower         TEXT NOT NULL DEFAULT 'none'
                       CHECK (watchtower IN ('none', 'registered', 'failed')),
    hns_lock_txid      TEXT,
    hns_lock_vout      INTEGER,
    -- The taker's own request, to check the maker's answer against.
    request_json       TEXT,
    -- The Buyer's secret nonce; NULL for the Seller.
    secret_nonce_hex   TEXT,
    -- When the trade first reached Completed, Refunded or Expired; it is
    -- still followed for a day after (a reorg can undo a last step).
    terminal_at        INTEGER,
    finished           INTEGER NOT NULL DEFAULT 0,
    -- 1 once this party's Commit is stored; never back to 0.
    committed          INTEGER NOT NULL DEFAULT 0,
    -- Bumped by every write: writers compare it (review C2), so a step that
    -- read the row before a Commit cannot write the old row back over it.
    version            INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    PRIMARY KEY (profile_id, trade_id)
);

-- Where each relay key's inbox was read up to.
CREATE TABLE IF NOT EXISTS exchange_inbox (
    relay_pubkey  TEXT PRIMARY KEY,
    profile_id    TEXT NOT NULL,
    cursor        TEXT
);
