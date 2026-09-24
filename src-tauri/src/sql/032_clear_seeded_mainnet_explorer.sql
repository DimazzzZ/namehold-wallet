-- Clear the mainnet explorer URL that migration 009 used to seed.
--
-- 009 originally seeded `explorer_api_url` with the mainnet explorer. It was
-- later changed to seed an empty string, so the runtime resolves the explorer
-- per the active profile's network (`Network::default_explorer_base_url`, G2)
-- and a testnet/regtest profile has no mainnet URL to inherit. That edit only
-- helps a database created after it: on an installation where 009 had already
-- run, the mainnet URL stayed in `settings`, and an explicit `explorer_api_url`
-- outranks the network default — so a testnet or regtest profile kept reading
-- from the mainnet explorer, which is the silent cross-network read the guard
-- exists to prevent.
--
-- Only the exact value 009 seeded is removed. A URL the user typed themselves
-- is theirs, is not necessarily mainnet, and is left alone; the network guard
-- in `providers::explorer_client_from_settings` refuses it when it disagrees
-- with the profile's network.
DELETE FROM settings
 WHERE key = 'explorer_api_url'
   AND value = 'https://e.hnsfans.com';
