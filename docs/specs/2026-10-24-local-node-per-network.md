# Local node per network

**Status:** accepted (2026-10-24), not implemented. Decided in [ADR-0005](../adr/0005-one-local-hsd-per-network.md). Amends [ADR-001](../adr/0001-per-profile-node-configuration.md) (Interaction with N11) and N11 of [2026-09-14 Network-derived behaviour](./2026-09-14-network-derived-behaviour.md). Builds on [2026-09-15 Per-profile node banner & preflight](./2026-09-15-per-profile-node-banner-and-preflight.md) for what the banner says.

## 1. Summary

A user with wallets on more than one network (a mainnet wallet and a regtest one, say) switches between them without touching the node. Each network gets its own local hsd, started the first time a wallet of that network becomes active and kept running until the app quits, so switching back is instant and a mainnet node keeps up with the chain while the user is on regtest. The app stops only the nodes it started; a node the user runs outside Namehold is used, never stopped. Full or SPV, and whether to start the node on its own, are chosen per network in Settings.

hsd facts here are pinned to hsd 8.0.0; paths are relative to its `lib/`.

## 2. Terms

- **Local node of a network** — the hsd that serves wallets of that network whose `chain_source` is `local_node` (full or SPV, by the network's mode): at `127.0.0.1:<the network's RPC port>`, chain in the network's data directory (`network_scoped_data_dir`). At most one per network.
- **Own node** — a local node the app started: a child of this app session, or the process named by the network's pid file when it is hsd with the app's `--prefix` and `--network` (R6).
- **Outside node** — a local node that answers on the network's port on the right chain but is not an own node: the user's hsd, or one a test run left. Used, never stopped.
- **Network node settings** — per network: mode (`full` or `spv`) and autostart (on or off).

## 3. Requirements

Every requirement names the code that will enforce it and the test that will pin it. The names are planned; `scripts/check-spec-refs.sh` is run on this spec once it is implemented, and a renamed test changes its line here in the same PR.

### 3.1 Which node a wallet reaches

**R1 — A local-node wallet without an override reaches its own network's port over a default global URL.** For a profile whose effective `chain_source` is `local_node` and whose own override supplies no URL, the effective URL is `Network::default_rpc_url()` of the profile's network when the global `node_rpc_url` is only a default: empty, or a loopback URL (`127.0.0.1`, `localhost`, `[::1]`) on some network's default RPC port, which is what the network started last left there. A global URL that is not a default (another host, a port no network uses by default) is the user's choice and is kept, so a node elsewhere on the LAN is never replaced by 127.0.0.1. A per-profile URL override still wins (ADR-001). A `remote_node` or `explorer` profile keeps the global URL. hsd's per-network RPC ports are 12037, 13037, 14037 and 15037 (`protocol/networks.js:557, 789, 954, 1123`), already carried by `Network::default_rpc_port`.
*Enforced:* `noncustodial/node_config.rs::effective_node_config_for_profile`, `noncustodial/rpc.rs::is_default_loopback_rpc_url`.
*Pinned:* `node_config_tests::{a_local_node_profile_reaches_its_networks_port_over_a_default_global_url, a_local_node_profile_keeps_a_global_url_that_is_not_a_default, a_remote_node_profile_keeps_the_global_url, a_profile_url_override_still_wins, an_override_equal_to_global_is_still_the_users_choice, overriding_the_chain_source_alone_still_reaches_the_networks_port, an_empty_url_override_is_no_choice_of_url, a_local_node_profile_with_nothing_set_reaches_its_networks_default_port, for_profile_of_a_local_node_profile_talks_to_its_networks_port}`, `rpc::tests::{a_loopback_url_on_a_default_port_is_a_default, a_custom_port_or_another_host_is_not_a_default}`.

**R2 — Nothing rewrites the global URL.** `start_hsd` writes no setting: R1 makes a rewrite unnecessary for a local node, and with several nodes it would flip a setting other profiles read. `realign_loopback_rpc_url`, the one function that wrote it, is removed.
*Enforced:* `commands/node.rs::start_hsd`.
*Pinned:* `node_lifecycle_tests::starting_a_node_leaves_the_global_url_as_it_was` (a remote-node profile over another network's loopback default, the case the realign still wrote once R1 was in).

### 3.2 One node per network

**R3 — Nodes of different networks run side by side.** The app keeps one own-node handle per network (`AppState`), and starting a network's node never stops another network's. hsd gives every network distinct RPC, p2p, brontide, wallet and DNS ports (`protocol/networks.js`: p2p 12038 / 13038 / 14038 / 15038, ns 5349 / 15349 / 25349 / 35349, rs 5350 / 15350 / 25350 / 35350), and each network has its own data directory, so two networks never share a port or a chain lock.
*To enforce:* `commands/node.rs::{start_node, stop_node}`, `AppState::hsd_children`.
*To pin:* `node_lifecycle_tests::two_networks_run_side_by_side`, `live_node_it::live_regtest_and_simnet_nodes_run_side_by_side`.

**R4 — A network's node starts when one of its wallets becomes active.** When a profile is activated (`activate_profile`, the one activation path) and at app start for the active profile, the app starts that profile's network node if the profile's `chain_source` is `local_node`, the network's autostart is on, and no own node or outside node of that network answers already. An outside node on the right chain is adopted for reading, as `start_hsd` does today; one on another chain is refused as today (`guard_probe_network`). A `remote_node` or `explorer` profile starts nothing, in either mode. The activation itself never waits for the node: it returns, and the banner shows the node starting (R11).
*To enforce:* `commands/secure_wallet.rs::activate_profile`, `commands/node.rs::ensure_network_node`, the startup hook in `lib.rs`.
*To pin:* `node_lifecycle_tests::{activating_a_local_node_wallet_starts_its_networks_node, activating_a_remote_node_wallet_starts_nothing, autostart_off_starts_nothing, an_answering_node_is_not_started_twice}`.

**R5 — Switching wallets stops nothing.** Activating a profile of another network leaves every running node as it is. A node lives until the user stops it (R8) or the app quits (R9).
*To enforce:* `commands/secure_wallet.rs::activate_profile`.
*To pin:* `node_lifecycle_tests::switching_to_another_network_leaves_the_first_node_running`.

### 3.3 Own and outside nodes

**R6 — An own node is recognised across sessions by its pid file.** At spawn the app writes `<data_dir>/namehold-hsd.pid` with the child's pid, and removes it when it stops the node. A node is own when it is this session's child, or when the pid file names a live process whose command line is the hsd binary with `--prefix=<the app's base>` and `--network=<the network>`. A pid file naming a dead process, or a process that is not that hsd (pids are reused), is stale: it is ignored and removed. Reading the command line is per platform (`/proc/<pid>/cmdline` on Linux, `sysctl KERN_PROCARGS2` on macOS, the process snapshot on Windows); a platform where it cannot be read treats the node as outside, never as own.
*To enforce:* `commands/node.rs::{write_pid_file, own_node_pid, is_our_hsd_cmdline}`.
*To pin:* `node_lifecycle_tests::{a_node_left_by_the_last_session_is_own, a_stale_pid_file_is_ignored_and_removed, a_reused_pid_running_something_else_is_not_own}`, `node_tests::is_our_hsd_cmdline_matches_only_our_prefix_and_network`.

**R7 — An outside node is never stopped.** Stop, the tray and quit act only on own nodes. For an outside node the Stop button is disabled with "This node was not started by Namehold, so Namehold does not stop it.", the tray item reads the same, and `stop_hsd` sends no RPC `stop` to it (today it sends one to whatever answers on the profile's URL).
*To enforce:* `commands/node.rs::stop_node`, `src/components/Settings.tsx`, the tray menu in `lib.rs`.
*To pin:* `node_cmd_tests::stopping_an_outside_node_sends_nothing` (mockito `.expect(0)` on the `stop` call), `settings-network-nodes.test.tsx` for the disabled button and its sentence.

### 3.4 Controls

**R8 — Start and Stop act on the active wallet's network and name it.** Settings and the tray offer one Start/Stop, for the active profile's network: "Start mainnet node", "Stop regtest node". Other networks' nodes are not listed or touched; to stop the mainnet node the user switches to a mainnet wallet. With no active profile there is no network, and Start is disabled with the sentence `start_hsd` already returns.
*To enforce:* `commands/node.rs::{start_hsd, stop_hsd}`, `src/queries/node.ts`, `src/components/Settings.tsx`, the tray menu in `lib.rs`.
*To pin:* `node_cmd_tests::start_and_stop_act_on_the_active_profiles_network`, `settings-network-nodes.test.tsx`.

**R9 — Quit keeps or stops every own node by one rule.** At quit, with background sync on, every own node is left running for the daemon, which syncs every profile; with it off, every own node is stopped. Outside nodes are left alone either way (R7). The pid file stays with a node left running, so the next session finds it own (R6).
*To enforce:* the exit handler in `lib.rs`.
*To pin:* `node_lifecycle_tests::{quit_with_background_sync_leaves_every_own_node, quit_without_background_sync_stops_every_own_node}`.

**R10 — Mode and autostart are settings of the network.** Settings shows a block per network that has at least one profile: mode (`Full node` / `SPV`) and "Start automatically". Every reader of the mode takes the network's: the spawn flags (`--spv`, or `--index-address --index-tx`) in `start_hsd`, the chain source derived from `chain_source` plus mode (`ChainSource::from_settings`: `local_node` + `spv` is `SpvNode`), and the SPV branches of sync, send and chain scan. They read it through one resolver of the profile's network, never the global `node_mode`. The keys are `node_mode.<network>` and `autostart_hsd.<network>` in `settings`. A migration seeds every network's keys from the current global `node_mode` and `autostart_hsd`, so nobody's node changes on upgrade, and the global keys are then no longer read. SPV keeps its chain in `spvchain/` and a full node in `chain/` (`blockchain/chain.js:4012`), so changing a network's mode never overwrites the other chain.
*To enforce:* `noncustodial/rpc.rs::{resolve_node_mode_for_network, ChainSource::from_settings}`, its callers in `commands/{node,sync,tx,chain_scan}.rs`, `db/migrations.rs`, `src/components/Settings.tsx`, `src/types/index.ts`.
*To pin:* `migration_tests::network_node_settings_are_seeded_from_the_global_ones`, `node_cmd_tests::the_networks_mode_picks_the_spawn_flags`, `node_config_tests::a_profiles_chain_source_follows_its_networks_mode`, `settings-network-nodes.test.tsx`.

### 3.5 What the user sees

**R11 — The status strip says the node is starting.** Between the spawn and the first RPC answer, the node status in the status strip reads "Starting <network> node…" instead of "unreachable". The other states (syncing, network mismatch, unreachable) are unchanged.
*To enforce:* `commands/node.rs::node_status` (a `starting` field), `src/components/ui/StatusStrip.tsx`.
*To pin:* `node_status_tests::a_spawned_node_not_answering_yet_is_starting`, `status-strip.test.tsx`.

**R12 — The daemon never starts or stops hsd.** `namehold-syncd` reads the nodes that run; a profile whose node does not answer is skipped until the next cycle, as today. SECURITY.md's statement of what the daemon does stays true.
*To enforce:* `daemon/mod.rs` (no spawn path).
*To pin:* `shakedex_layering_tests`-style source check that `daemon/` names no spawn of hsd.

## 4. Explicitly not enforced

- **No idle stop.** A node whose wallets have not been active for hours keeps running until quit.
- **No list of all running nodes.** Only the active network's node has controls; a running node of another network is visible only by switching to one of its wallets.
- **Not a supervisor.** A node that crashes is not restarted on its own; the next activation of one of its wallets, or Start, starts it again.
- **The daemon does not bring nodes back after a reboot.** Background sync with the app closed works only for nodes still running.

## 5. Known gaps

- **Two full nodes cost memory and CPU.** Accepted: regtest and simnet are cheap, and the costly mainnet node is one the user already ran.
- **A custom global URL serves every local-node profile alike.** A global URL that is not a default is kept for every network's local-node profiles, so two networks pointed at one custom global URL still reach one node; the network guard refuses the wrong chain, and a per-profile override is the way to give each its own. Accepted: it is an explicit setting, and replacing it would reroute a user who configured a node by hand.

## 6. Implementation plan

Each step is one PR-sized change with its tests, and leaves the app working on its own.

| Step | Change | Requirements | Status |
|------|--------|--------------|--------|
| 1 | A `local_node` profile without an override reaches its network's port over a default global URL; a custom global URL is kept; `start_hsd` stops writing `node_rpc_url` and the realign is removed | R1, R2 | Done |
| 2 | Mode and autostart per network: migration, one resolver of the profile's network, every reader of the mode switched to it | R10 (backend) | Not started |
| 3 | Pid file and own-node recognition by command line (macOS, Linux, Windows) | R6 | Not started |
| 4 | One own-node handle per network in `AppState`; Start/Stop per network; an outside node is never stopped | R3, R7 | Not started |
| 5 | Start on wallet activation and at app start; switching stops nothing | R4, R5 | Not started |
| 6 | Quit by one rule for every own node; source check that the daemon names no hsd spawn | R9, R12 | Not started |
| 7 | UI: per-network node block in Settings, Start/Stop and tray labels naming the network, "Starting…" in the status strip | R8, R10 (UI), R11 | Not started |
| 8 | Live regtest + simnet side-by-side test, `docs/NODE_SETUP.md`, CHANGELOG | R3 (live), docs | Not started |

## 7. Pointers

- Code: `commands/node.rs`, `commands/secure_wallet.rs::activate_profile`, `noncustodial/node_config.rs`, `lib.rs` (startup, tray, exit), `src/components/Settings.tsx`, `src/components/ui/StatusStrip.tsx`.
- Tests: `node_lifecycle_tests`, `node_cmd_tests`, `node_config_tests`, `node_status_tests`, `migration_tests`, `live_node_it`, `settings-network-nodes.test.tsx`, `status-strip.test.tsx`.
- Docs: `docs/NODE_SETUP.md` (one node per network, outside nodes), `CONTEXT.md` (terms above), CHANGELOG `[Unreleased]`.
