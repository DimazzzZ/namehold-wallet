-- Correct the external read API URL for existing installs.
--
-- Earlier builds seeded `external_read_api_url` as `https://hnsfans.com`, but
-- the documented explorer API (the `/api/addresses`, `/api/names`, and
-- `/api/txs` routes the app relies on) is served from `https://e.hnsfans.com`.
-- Using the bare host returned no usable data, which surfaced as a false zero
-- balance for external read-only wallets.
--
-- This migration rewrites ONLY the stale default value so that any URL a user
-- explicitly customized is left untouched.
UPDATE settings
   SET value = 'https://e.hnsfans.com'
 WHERE key = 'external_read_api_url'
   AND value = 'https://hnsfans.com';
