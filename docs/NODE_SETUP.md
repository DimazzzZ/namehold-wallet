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
- **Background sync:** When "Sync in background" is enabled in Settings → Connections
  (default ON), hsd stays running after the app closes.
  A background daemon (`namehold-syncd`) wakes every 60 seconds to sync wallet
  data from hsd into the local database. The next app launch adopts the running
  hsd. To stop hsd, disable "Sync in background" or manually click **Stop hsd**
  in Settings → Connections.
- Then in the app: **Settings → Node RPC** → URL `http://127.0.0.1:12037`, API key
  `<your-key>`; click **Sync** to pull your UTXOs; **Send** is now enabled.

While the node is down you can still view balance/names (explorer); the app shows
"Start your local node to send" and disables spend actions until it's reachable.

## Remote node (point at someone else's hsd)

Instead of running hsd locally, you can point the wallet at an existing hsd RPC:

- **On first run:** the "How do you want to connect?" step offers "Remote node" as a choice.
  Enter the RPC URL (e.g., `https://node.example.com:12037`) and optional API key, then
  click "Test connection" to validate it before committing.
- **In Settings:** go to Connections → Chain source → select "Remote node", enter the URL
  and API key, and click "Test connection" to verify.
- **Stored API key:** the Settings API-key field is write-only (a stored key is never shown
  back), so "Test connection" reuses your stored key when the URL you probe matches the
  saved node. A freshly typed URL is probed without the stored key — the secret is never
  sent to an endpoint you just typed.
- **Network mismatch:** "Test connection" compares the node's reported network with the
  wallet's and flags a mismatch (e.g. a testnet node for a mainnet wallet). A mismatched
  node is refused everywhere: the app will not read from it, will not sync from it, reports
  it as unable to send, and refuses to broadcast through it — the signed transaction never
  leaves your device. Settings will not save one either. This applies during first-run
  onboarding too: there is no wallet profile yet, so the comparison uses the network you
  picked on the previous step.
- **To send:** enable "Allow sending via remote node" (off by default for safety; shown in
  the onboarding Remote step and in Settings → Connections). This sets the
  `allow_remote_broadcast` flag, which gates the broadcast path. Your recovery phrase
  never leaves this device — remote is a privacy/trust tradeoff, not custody.
- **Plaintext-key guard:** the app refuses to send an API key over plaintext HTTP to a
  non-loopback host (e.g., `http://example.com`). Use `https://` for remote nodes or
  `http://127.0.0.1` for local testing.

Remote nodes are ideal for:
- Quick setup without syncing 15GB locally.
- Running the wallet on a low-power device (phone, tablet, old laptop).
- Trusting a node operator you know (family, friend, business partner).

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

## Where data and node config live

The wallet keeps **all of its own state in a single directory, `~/.namehold`** —
not the OS-specific app-data dir. The bundle identifier
(`org.zhavoronkov.nameholdwallet`) is kept only for packaging/signing and no
longer dictates where data lives. `~/.namehold` is deliberately the "pair" of
the node's `~/.hsd`: wallet data under `~/.namehold`, node (chain) data under
`~/.hsd`.

There is **one `~/.namehold` for all networks and profiles** — it is not split
per network. Everything per-network (mainnet/testnet/regtest) lives either as
rows inside the shared database or in the node's own datadir.

| Path | What it holds |
|------|---------------|
| `~/.namehold/portfolio.db` | The shared SQLite database: wallet profiles, portfolio (UTXOs, name states, transactions), and **all node configuration** (global + per-profile `node_rpc_url`, `node_rpc_api_key`, `chain_source`). Used by both the GUI and the `namehold-syncd` daemon. |
| `~/.namehold/syncd.pid` | PID file for the background sync daemon (see above). |

**Node RPC config is not stored in files.** The connection settings you enter
under **Settings → Connections** are written as rows in `portfolio.db` (a global
default, plus per-profile overrides), not as a per-network config file. Each key
resolves on its own: per-profile override → global setting → built-in default.
Splitting the read and write sides so a profile could read through one source
and send through another is planned, not built — see step 8 of
`docs/specs/2026-09-15-per-profile-node-banner-and-preflight.md`.

### When the wallet calls a node "synced"

The chain tip is the test: the node is synced once the blocks it has applied
have caught up to the best header it knows about. `verificationprogress` only
corroborates that, because it can plateau just below 1.0 — around 0.9997 on
regtest — and a node sitting at the tip would otherwise never qualify. To keep
a node that reports `blocks == headers` while barely verified from passing,
progress (when reported at all) must also clear a loose 0.999 floor.

Two fallbacks matter in practice:

- **No header height reported.** Older builds and some nodes do not send one.
  The wallet falls back to `verificationprogress >= 0.9999`.
- **Neither reported.** A regtest node with a single miner reports no sync
  metadata at all. A gate on a node you configured assumes synced, so regtest
  keeps working; a first-contact probe of a node you have just typed in assumes
  the opposite, so an unknown node is not trusted on no evidence.

One rule answers this everywhere — the read gate, the write gate, the node
status panel and "Test connection" — so the label in Settings cannot disagree
with what reads actually do.

### Node (hsd) datadirs and default RPC ports

The node's chain data is per-network and lives under `~/.hsd` (or a custom
`--prefix`). The wallet's built-in RPC defaults per network are:

| Network | Default node datadir | Default RPC URL |
|---------|----------------------|-----------------|
| mainnet | `~/.hsd/` | `http://127.0.0.1:12037` |
| testnet | `~/.hsd/testnet/` | `http://127.0.0.1:13037` |
| regtest | `~/.hsd/regtest/` | `http://127.0.0.1:14037` |
| simnet | `~/.hsd/simnet/` | `http://127.0.0.1:15037` |

- The datadir the app uses for hsd comes from the `hsd_prefix` setting when set;
  otherwise it falls back to `~/.hsd`.
- `scripts/regtest.sh` is the exception: for reproducible, throwaway testing it
  runs hsd with `--prefix=<repo>/.regtest` (git-ignored) and `--api-key=test`, so
  test runs never touch your real `~/.hsd` chain. See `REGTEST_TESTING.md`.
- Only mainnet has a built-in public explorer (`e.hnsfans.com`); testnet, regtest
  and simnet have no explorer fallback.

## SPV mode (lightweight alternative)

For users who don't need to send transactions or want faster initial setup:

- **SPV mode** runs hsd with `--spv` (no `--index-address`), downloading only block
  headers. Much faster initial sync and minimal disk usage (~几十MB vs ~15GB).
- **Read-only** — cannot send transactions. Balance and name data come from the
  explorer.
- **Explorer failover** — set a fallback URL in Settings for when the primary
  explorer is unreachable.
- **To enable:** Settings → Connections → Chain source → select "Local node (SPV)"
  → Save. There is no separate "Node mode" dropdown; the one selector sets both
  the source and, for a local node, whether it runs full or SPV.
- **Status indicator:** StatusStrip shows "Explorer (SPV)" when SPV mode is active.

SPV mode is ideal for:
- **Viewing your wallet** without waiting for full sync
- **Monitoring names/auctions** without needing to send
- **Low-disk environments** where full node storage isn't feasible
- **Quick setup** for new users who want to see their balance immediately

To send transactions, switch back to "Full node" mode and wait for sync.
