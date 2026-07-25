-- Namebase session cookie: move from plaintext to encrypted-at-rest.
--
-- The cookie is a bearer credential for the user's Namebase custodial account.
-- Previously stored in plaintext in the `settings` table under key
-- `namebase_cookie`. This migration adds a new key `namebase_cookie_v1` which
-- holds a hex-encoded AES-256-GCM blob encrypted under an OS-keyring-held DEK.
--
-- The plaintext key is NOT deleted here — a follow-up migration (after one
-- release cycle) will drop it. This allows the read path to migrate existing
-- plaintext cookies to the new format on first access, then blank the legacy
-- row in the same transaction.

INSERT OR IGNORE INTO settings (key, value) VALUES ('namebase_cookie_v1', '');
