# Node setup — reading vs. sending

This wallet is **non-custodial**: it holds your keys locally and signs on your
device. It splits how it talks to the Handshake network:

- **Reading (balance + names): no node needed.** Balances and name info are read
  from the **HNSFans explorer** (`https://e.hnsfans.com`) using your wallet's own
  addresses. You can view your wallet immediately after creating/importing it.
- **Sending (HNS sends + name covenant actions): needs an hsd node, local or remote.** Spending
  requires the wallet's unspent-coin set (UTXOs) and the ability to broadcast a
  Handshake-format transaction — and **no hosted provider offers that**. We checked:
  - HNSFans / 3xpl — read-only, no UTXOs, no broadcast.
  - Blockchair — does not support Handshake at all (no API for it).
  - HNScan — no public API. Namebase — custodial, shut down on 1 October 2026.

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
- **Autostart is on by default, per network.** When a wallet that uses a local node becomes active (at launch, or when you switch to it), the app starts that network's hsd for you. Turn it off for the active wallet's network under Settings › Node & tray → "Start the mainnet node automatically" (each network keeps its own setting; a network you never set follows the app-wide one). If a node already answers on that network's port on the right chain, the app uses it instead of spawning a new one. A node you stopped stays stopped until the active wallet changes.
- **Background sync:** When "Sync in background" is enabled in Settings › Node & tray (default ON), every node the app started stays running after the app closes, one per network. A background daemon (`namehold-syncd`) wakes every 60 seconds to sync wallet data from those nodes into the local database; it never starts or stops a node itself. On macOS and Linux the next app launch recognises those nodes as its own from `namehold-hsd.pid` in each network's data directory (the pid and the process start time); on Windows it does not, so a node left running is used but not stopped by the app after a restart. With "Sync in background" off, closing the app stops every node it started, including, on macOS and Linux, ones left from an earlier session. To stop a network's node by hand, switch to one of its wallets and click **Stop mainnet node** (or the network you are on) in Settings › Node & tray or in the readiness pill.
- **The node the app starts always has an API key.** The app starts hsd with the key configured in Settings → Connections (or the `api-key` of `hsd.conf` in the configured data directory). With neither, it generates a new random key for each start of a network's node and keeps it in its own database (never shown in Settings). The key reaches hsd through the `HSD_API_KEY` environment variable, never the command line, and `--no-auth=false` keeps it in force even if `hsd.conf` says `no-auth`. The generated key is sent only to that network's default RPC port on `127.0.0.1` (or `localhost`), and only while the node the app started is running, by the pid and start time the app recorded when it started it: never to a remote node, another port, or a node on that port that the app did not start. On a system where the app cannot read a process's start time (Windows), it cannot tell these apart, so it does not generate a key: set one in Settings → Connections before starting a node. hsd's own wallet, which the started node also runs, takes the same key unless your `hsw.conf` sets one of its own. To use the started node from `hsd-cli` or `hsw-cli`, set your own key in Settings → Connections, then stop and start the node: it then runs with yours. The started node listens on `127.0.0.1` whatever `hsd.conf` says (`--http-host=127.0.0.1`). If something else holds the network's RPC port and does not answer as a usable node, the app refuses to start one and says so: stop what holds the port, or, if it is your own hsd, set its key in Settings.
- **Logs of the node the app starts.** hsd runs at log level `info` (its default is `debug`, which records every address the wallet looks up and every raw transaction it sends), and on macOS and Linux with umask 077, so the files it creates (its `debug.log`, chain, key) are readable by your account only. Its output goes to `namehold-hsd.log` in the network's data directory (0600); at each start the previous one becomes `namehold-hsd.log.1`, and logs an earlier node left there (`debug*.log`, `namehold-hsd.log.1`) are made 0600 too. A data directory the app creates is 0700; an existing one keeps its mode.
- Then in the app: **Settings › Connections**: Node RPC URL (sending) `http://127.0.0.1:12037`, Node RPC API key `<your-key>`, **Save settings** and confirm in the secure window; click **Sync** to pull your UTXOs; **Send** is now enabled.

While the node is down you can still view balance/names (explorer); the app shows
"Start your local node (<url>) to send." and disables spend actions until it's reachable.

## Remote node (point at someone else's hsd)

Instead of running hsd locally, you can point the wallet at an existing hsd RPC:

- **On first run:** the "How do you want to connect?" step offers "Remote node" as a choice.
  Enter the RPC URL (e.g., `https://node.example.com:12037`) and optional API key, then
  click "Test connection" to validate it before committing.
- **In Settings:** go to Connections → Chain source → select "Remote node", enter the URL
  and API key, and click "Test connection" to verify.
- **Stored API key:** the Settings API-key field is write-only (a stored key is never shown
  back), so "Test connection" reuses your stored key when the URL you probe matches the
  saved node. A freshly typed URL is probed without any stored or generated key — the secret
  is never sent to an endpoint you just typed.
- **Network mismatch:** "Test connection" compares the node's reported network with the
  wallet's and flags a mismatch (e.g. a testnet node for a mainnet wallet). A mismatched
  node is refused everywhere: the app will not read from it, will not sync from it, reports
  it as unable to send, and refuses to broadcast through it — the signed transaction never
  leaves your device. A send also stops when the node's network cannot be confirmed (the node
  does not answer, or does not name its chain). Settings will not save a mismatched node either. This applies during first-run
  onboarding too: there is no wallet profile yet, so the comparison uses the network you
  picked on the previous step.
- **To send:** enable "Allow sending via remote node" (off by default for safety; shown in
  the onboarding Remote step and in Settings → Connections). This sets the
  `allow_remote_broadcast` flag, which gates the broadcast path. Your recovery phrase
  never leaves this device — remote is a privacy/trust tradeoff, not custody.
- **Any node off this machine counts as remote:** the opt-in is asked for whenever the node URL's host is not this computer, even if the chain source says local node. Sending shows as unavailable, and a send is refused at broadcast, until you choose Remote node and enable the opt-in.
- **A full node, not SPV:** the remote hsd must be a full node with the same indexes as a local one (`--index-tx --index-address`). An hsd started with `--spv` keeps no name state and no coins: it answers `getnameinfo` with no state for every name and cannot answer a coin lookup, so the wallet sees no auction on any name and cannot confirm a stranded lockup, an unrevealed bid or a balance against it.
- **Plaintext-key guard:** the app refuses to send an API key over plaintext HTTP to a
  non-loopback host (e.g., `http://example.com`). A remote node must use `https://`, with or without a key; `http://` is for `127.0.0.1` and other loopback addresses.

Remote nodes are ideal for:
- Quick setup without syncing 15GB locally.
- Running the wallet on a low-power computer.
- Trusting a node operator you know (family, friend, business partner).

## Changing connection settings

The settings that decide which node and explorers the wallet trusts, which program it runs and how long an unlocked wallet stays unlocked are saved only after you confirm them in Namehold's secure window: the node RPC URL and API key, the chain source and node mode (each network's own mode too), "Allow sending via remote node", the explorer URL and its fallback, the hsd program path, the hsd data directory, and the signer session timeout. Saving opens the window with one line per change, old value → new value; an API key is never shown, only whether it is set, changed or cleared. Cancel there and nothing is saved. Saving values that equal the stored ones asks nothing.

- **hsd program path:** empty means auto-detect. Otherwise it must be the full path of an executable file that only its owner can change, owned by you or by root; the window shows, and the app keeps, the file the path leads to (a link is followed). A stored path that does not meet this is not run: the node card says why, and Start refuses. Because the file itself is kept, a Homebrew path such as `/opt/homebrew/bin/hsd` is stored under one `Cellar/hsd/<version>` directory and stops working after `brew upgrade hsd`: leave the path empty for Homebrew, or confirm it again after an upgrade.
- **hsd data directory:** empty means hsd's default (`~/.hsd`). Otherwise a full path to a directory (it may not exist yet), not a file, and not on a network share.
- **URLs:** `https://`, or `http://` only for this computer (`127.0.0.1`, `::1`, `localhost`). Write the host plainly, with no user name or password in the URL, no percent-encoding and no numeric spelling such as `127.1`, so the line you confirm names the host the app connects to; an international host name goes in its `xn--` form. The plaintext-key guard above applies when you save, too. A URL saved before this rule keeps working until you change it.
- **API key and a new host:** the stored key goes to whatever host the node URL names. When a new host would receive it, the window adds a warning that names the host; to avoid that, clear or replace the key in the same save.

## Regtest (for testing the full send/name flows)

See `REGTEST_TESTING.md` — run `hsd --network=regtest --index-address --index-tx
--api-key=test` (RPC on `:14037`), create a regtest wallet, mine to your receive
address, then exercise send + the covenant actions end-to-end.

## Background sync daemon

When "Sync in background" is enabled (Settings › Node & tray, default ON):

- A separate Rust binary `namehold-syncd` runs every 60 seconds, syncing all
  wallet profiles (UTXOs, name states, transactions) from hsd into the shared
  SQLite database.
- hsd stays running after the app closes (not killed). The daemon keeps it alive
  so it can sync in the background.
- The background daemon never signs or broadcasts; it publishes listings already signed to LearnHNS Market.
- A cross-process DB lock table (`sync_locks`) coordinates the app's manual Sync and the daemon, using heartbeats (every 10 seconds) and stale-lock takeover (after 30 seconds) to prevent conflicts. The app's Sync takes the lock from a daemon run; a run whose lock is taken over sees it at its next heartbeat (within 10 seconds) and stops at its next check; the LearnHNS Market jobs read the lock row before each listing, so they stop at once.
- The daemon writes its PID to `~/.namehold/syncd.pid`.
- **Crash recovery:** if the daemon dies, the app respawns it on startup (if the
  toggle is ON).

To disable background sync, uncheck **Settings › Node & tray → "Sync in background"**. hsd will be stopped the next time you close the app.

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

### One node per network

Wallets on different networks do not share a node. Each network gets its own hsd, on its own ports and in its own data directory (the table above), and they run side by side: switching from a regtest wallet to a mainnet wallet starts the mainnet node and leaves the regtest node running, so switching back is instant.

- **Which URL a local-node wallet uses.** A wallet whose chain source is a local node, with no node URL of its own, reaches its network's default port (mainnet 12037, testnet 13037, regtest 14037) whenever the app-wide node URL is only a default: empty, or a loopback URL on some network's default port. An app-wide URL you set by hand (another machine on your network, a custom port) is used as is, for every local-node wallet. A per-wallet node URL always wins. Nothing rewrites the app-wide URL when a node starts.
- **Nodes you run yourself.** A node that answers on the network's port on the right chain but that Namehold did not start is used, never stopped: its Stop button is disabled with "This node was not started by Namehold, so Namehold does not stop it.", and quitting leaves it alone.
- **Full or SPV is per network.** The Chain source selector sets it for the active wallet's network; your other networks keep theirs.

## SPV mode (lightweight alternative)

For users who don't need to send transactions or want faster initial setup:

- **SPV mode** runs hsd with `--spv` (no `--index-address`), downloading only block
  headers. Much faster initial sync and minimal disk usage (tens of MB vs ~15GB).
- **Read-only** — cannot send transactions. Balance and name data come from the
  explorer.
- **Explorer failover** — set a fallback URL in Settings for when the primary
  explorer is unreachable.
- **To enable:** Settings → Connections → Chain source → select "SPV — lightweight, read-only (headers only, explorer for data)" → Save settings. There is no separate "Node mode" dropdown; the one selector sets both the source and, for a local node, whether the active wallet's network's node runs full or SPV. Each network keeps its own choice.
- **Status indicator:** the readiness pill's panel reads "Reads from the explorer (SPV)" when SPV mode is active.

SPV mode is ideal for:
- **Viewing your wallet** without waiting for full sync
- **Monitoring names/auctions** without needing to send
- **Low-disk environments** where full node storage isn't feasible
- **Quick setup** for new users who want to see their balance immediately

To send transactions, switch back to "Local full node (this device runs hsd)" and wait for sync.
