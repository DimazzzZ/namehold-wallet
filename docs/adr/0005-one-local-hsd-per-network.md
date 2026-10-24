# One local hsd per network, reached on its own network's port

Status: accepted (2026-10-24). Amends [ADR-001](./0001-per-profile-node-configuration.md) (its "Interaction with N11" section) and N11 of the [network-derived behaviour spec](../specs/2026-09-14-network-derived-behaviour.md).

The app runs at most one hsd it started (`AppState::hsd_child`), launched for the active profile's network. A user with a mainnet wallet and a regtest wallet who switches between them keeps the node of the wallet they left: the mainnet wallet then talks to a regtest node, or to nothing, until the user stops one node and starts the other by hand. Worse, the URL a profile without its own override uses is one global `node_rpc_url`, which `start_hsd` rewrites to the starting network's port (`realign_loopback_rpc_url`), so every switch would flip a setting every other profile reads.

We will run one local hsd per network, side by side. hsd gives every network its own ports (RPC 12037 / 13037 / 14037 / 15037, and its own p2p, brontide, wallet and DNS ports, `lib/protocol/networks.js`), and the app already gives every network its own data directory (`network_scoped_data_dir`), so two networks never share a port or a chain directory. A network's node is started when a wallet of that network becomes active and lives until the app quits; switching wallets stops nothing. A profile whose `chain_source` is `local_node` (full or SPV), and that has no URL override, reaches its network's node at `127.0.0.1:<that network's RPC port>`. The global `node_rpc_url` keeps its meaning only for a remote node, and nothing rewrites it any more.

Full or SPV, and whether to start the node on its own, become settings of the network rather than of the whole app: every wallet of a network shares its one node, so the mode cannot belong to a wallet, and a single global mode would force regtest and mainnet into the same one.

The app stops only a node it started. A node it did not start — the user's own hsd, or one a test run left — is used when it answers on the network's port on the right chain, as today, but no button and no quit stops it. A node the app started in an earlier session and left running for the background daemon still counts as the app's, recognised by a pid file in the network's data directory whose process is hsd with the app's prefix and network.

## Consequences

- `AppState` holds one child per network instead of one child. Status, Start and Stop take a network; the UI and the tray act on the active wallet's network and name it.
- `stop_hsd` no longer sends RPC `stop` to whatever answers on the port: a node outside Namehold is never stopped.
- A pid file `<data_dir>/namehold-hsd.pid` is written at spawn and removed at stop. A pid whose process is not hsd with the expected `--prefix` and `--network` is stale and ignored, since pids are reused.
- `node_mode` and `autostart_hsd` move from global settings to per-network settings. Their current values seed every network once, so nobody's behaviour changes on upgrade. Every reader of the mode (spawn flags, `ChainSource::from_settings`, the SPV branches of sync, send and chain scan) reads the profile's network's.
- The realign of ADR-001's N11 section stops writing settings. A user who had pointed the global URL at a non-default local port for a local node needs a per-profile override instead; the migration keeps such a URL as the override of every local-node profile it served, so the node they reach does not change.
- With background sync on, every node the app started is left running at quit, so the daemon can sync every profile; with it off, every node the app started is stopped. The daemon never starts or stops hsd.
- Two hsd processes cost memory and CPU. A regtest node is cheap; the cost that matters is a mainnet full node, which the user was already running.

## Related

- [Spec: local node per network](../specs/2026-10-24-local-node-per-network.md) — the requirements and their tests.
- [ADR-001](./0001-per-profile-node-configuration.md) — per-profile override still wins over everything here.
