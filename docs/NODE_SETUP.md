# Node setup — reading vs. sending

This wallet is **non-custodial**: it holds your keys locally and signs on your
device. It splits how it talks to the Handshake network:

- **Reading (balance + names): no node needed.** Balances and name info are read
  from the **HNSFans explorer** (`https://e.hnsfans.com`) using your wallet's own
  addresses. You can view your wallet immediately after creating/importing it.
- **Sending (HNS sends + name covenant actions): needs a local hsd node.** Spending
  requires the wallet's unspent-coin set (UTXOs) and the ability to broadcast a
  Handshake-format transaction — and **no hosted provider offers that**. We checked:
  - HNSFans / 3xpl — read-only, no UTXOs, no broadcast.
  - Blockchair — does not support Handshake at all (no API for it).
  - HNScan — no public API. Namebase — custodial/being wound down.

  So to send, run hsd yourself (locally, or on a small VPS — the chain is light).

## Run a node for sending

```sh
hsd --index-address --index-tx --api-key=<your-key>
# mainnet node RPC: http://127.0.0.1:12037
```

- `--index-address` is **required** (the wallet scans coins by address); `--index-tx`
  backs transaction history + confirmation tracking. The app starts hsd with both.
- **hsd cannot add an index to an already-synced chain** ("Cannot retroactively
  enable … indexing"). If your existing chain was synced without these indexes, it
  must be re-synced with them — the app detects this and offers a one-click re-sync
  (it moves the old `blocks/`, `chain/`, `tree/` aside and re-syncs).
- **Autostart is on by default.** The app starts hsd for you on launch (toggleable
  under Settings → Connections → "Autostart HSD when the app launches"). If hsd is
  already running when the app starts, it adopts the existing node via RPC instead
  of spawning a new one.
- **Background sync (since v0.3.0):** When "Sync in background" is enabled in
  Settings → Connections (default ON), hsd stays running after the app closes.
  A background daemon (`namehold-syncd`) wakes every 60 seconds to sync wallet
  data from hsd into the local database. The next app launch adopts the running
  hsd. To stop hsd, disable "Sync in background" or manually click **Stop hsd**
  in Settings → Connections.
- Then in the app: **Settings → Node RPC** → URL `http://127.0.0.1:12037`, API key
  `<your-key>`; click **Sync** to pull your UTXOs; **Send** is now enabled.

While the node is down you can still view balance/names (explorer); the app shows
"Start your local node to send" and disables spend actions until it's reachable.

## Regtest (for testing the full send/name flows)

See `REGTEST_TESTING.md` — run `hsd --network=regtest --index-address --index-tx
--api-key=test` (RPC on `:14037`), create a regtest wallet, mine to your receive
address, then exercise send + the covenant actions end-to-end.

## Background sync daemon

When "Sync in background" is enabled (Settings → Connections, default ON):

- A separate Rust binary `namehold-syncd` runs every 60 seconds, syncing all
  wallet profiles (UTXOs, name states, transactions) from hsd into the shared
  SQLite database.
- hsd stays running after the app closes (not killed). The daemon keeps it alive
  so it can sync in the background.
- The daemon is **read-only**: it never signs or broadcasts transactions.
- A cross-process DB lock table (`sync_locks`) coordinates the app's manual Sync
  and the daemon, using heartbeats (every 10 seconds) and stale-lock takeover
  (after 30 seconds) to prevent conflicts.
- The daemon writes its PID to `~/.namehold/syncd.pid`.
- **Crash recovery:** if the daemon dies, the app respawns it on startup (if the
  toggle is ON).

To disable background sync, uncheck **Settings → Connections → "Sync in
background"**. hsd will be stopped the next time you close the app.

## SPV mode (lightweight alternative)

For users who don't need to send transactions or want faster initial setup:

- **SPV mode** runs hsd with `--spv` (no `--index-address`), downloading only block
  headers. Much faster initial sync and minimal disk usage (~几十MB vs ~15GB).
- **Read-only** — cannot send transactions. Balance and name data come from the
  explorer.
- **Explorer failover** — set a fallback URL in Settings for when the primary
  explorer is unreachable.
- **To enable:** Settings → Connections → Node mode → select "SPV" → Save.
- **Status indicator:** StatusStrip shows "Explorer (SPV)" when SPV mode is active.

SPV mode is ideal for:
- **Viewing your wallet** without waiting for full sync
- **Monitoring names/auctions** without needing to send
- **Low-disk environments** where full node storage isn't feasible
- **Quick setup** for new users who want to see their balance immediately

To send transactions, switch back to "Full node" mode and wait for sync.
