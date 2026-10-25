# Local node per network

**Status:** accepted (2026-10-24), not implemented. Decided in [ADR-0005](../adr/0005-one-local-hsd-per-network.md). Amends [ADR-001](../adr/0001-per-profile-node-configuration.md) (Interaction with N11) and N11 of [2026-09-14 Network-derived behaviour](./2026-09-14-network-derived-behaviour.md). Builds on [2026-09-15 Per-profile node banner & preflight](./2026-09-15-per-profile-node-banner-and-preflight.md) for what the banner says.

## 1. Summary

A user with wallets on more than one network (a mainnet wallet and a regtest one, say) switches between them without touching the node. Each network gets its own local hsd, started the first time a wallet of that network becomes active and kept running until the app quits, so switching back is instant and a mainnet node keeps up with the chain while the user is on regtest. The app stops only the nodes it started; a node the user runs outside Namehold is used, never stopped. Full or SPV, and whether to start the node on its own, are chosen per network in Settings.

hsd facts here are pinned to hsd 8.0.0; paths are relative to its `lib/`.

## 2. Terms

- **Local node of a network** — the hsd that serves wallets of that network whose `chain_source` is `local_node` (full or SPV, by the network's mode): at `127.0.0.1:<the network's RPC port>`, chain in the network's data directory (`network_scoped_data_dir`). At most one per network.
- **Own node** — a local node the app started: a child of this app session, or the process named by the network's pid file when its start time is the recorded one (R6).
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
*Enforced:* `commands/node.rs::{start_hsd, stop_hsd, is_running}` over `AppState::hsd_children`; the tray snapshot and toggle (`commands/tray.rs`, `lib.rs`) read the active network's child, and quit drains every child.
*Pinned:* `node_lifecycle_tests::{two_networks_run_side_by_side, a_second_start_of_the_same_network_is_refused}`.
*To pin (step 8):* `live_node_it::live_regtest_and_testnet_nodes_run_side_by_side` (profiles are stored only for mainnet, testnet and regtest, so the pair is regtest and testnet).

**R4 — A network's node starts when one of its wallets becomes active.** Once a second the app looks at the active wallet; when it changed since the last look (the first look, at app start, counts as a change), and the wallet's `chain_source` is `local_node` and its network starts on its own (`autostart_hsd_for`), the app starts that network's node in the background with `start_hsd`. This catches every activation path (switching, creating, importing, Ledger) without the activation commands calling into `commands::node`. The activation never waits for the node, and the status strip shows it starting (R11). `start_hsd` adopts an outside node that answers on the right chain and refuses one on another chain, as before (`guard_probe_network`), and refuses only when this network's own node already runs (R3). A `remote_node` or `explorer` wallet starts nothing; before, the launch-time autostart started hsd for those too. The loop acts on a change only, so a node the user stopped is not started again until the active wallet changes: it is not a supervisor (§4).
*Enforced:* `commands/node.rs::{autostart_on_profile_change, node_to_start_on_activation}`, the autostart loop in `lib.rs` (which replaced the one-shot autostart at launch), `noncustodial/node_config.rs` (`EffectiveNodeConfig::local_node`).
*Pinned:* `node_lifecycle_tests::a_change_of_the_active_wallet_names_the_node_to_start`, `node_cmd_tests::{activating_a_local_node_wallet_names_its_networks_node, activating_a_remote_node_or_explorer_wallet_names_no_node}`.

**R5 — Switching wallets stops nothing.** Activating a profile of another network leaves every running node as it is: nothing on the activation path stops a node, and starting the new network's node leaves the others (R3). A node lives until the user stops it (R8) or the app quits (R9).
*Enforced:* `commands/node.rs::start_hsd` (one child per network).
*Pinned:* `node_lifecycle_tests::two_networks_run_side_by_side`.

### 3.3 Own and outside nodes

**R6 — An own node is recognised across sessions by its pid and start time.** hsd overwrites its own command line (`process.title = 'hsd'`), so a process's arguments cannot tell the app's hsd from another. At spawn the app writes `<data_dir>/namehold-hsd.pid` with the child's pid and its process start time as the OS reports it, and removes the file when it stops that node. A node is own when it is this session's child, or when the pid file names a live process whose start time is the recorded one: a reused pid belongs to a process that started at another time. A pid file naming a dead process, another start time, or that cannot be read is stale: it is ignored and removed. The start time is read with `proc_pidinfo(PROC_PIDTBSDINFO)` on macOS and field 22 of `/proc/<pid>/stat` on Linux; elsewhere it is not read, nothing is recorded, and an orphan counts as outside (§5).
*Enforced:* `commands/local_node.rs::{process_start_time, write_pid_file, own_node_pid, remove_pid_file}`, called from `commands/node.rs::{start_hsd, stop_hsd}`.
*Pinned:* `local_node_tests::{a_live_process_has_one_start_time, an_exited_process_has_no_start_time, a_pid_file_of_a_live_process_names_it, a_stale_pid_file_is_ignored_and_removed, no_pid_file_names_nothing}`, `node_lifecycle_tests::a_spawned_node_is_recorded_as_own_until_it_is_stopped`.

**R7 — An outside node is never stopped.** Stop, the tray and quit act only on own nodes. `stop_hsd` stops this session's child of the active network, or the own node its pid file names (SIGTERM, R6); a node that answers but is neither is refused with "This node was not started by Namehold, so Namehold does not stop it." and sent no RPC `stop` (before, `stop_hsd` sent one to whatever answered on the profile's URL). Until step 7 disables the button, Settings and the status strip show that refusal as an error toast; afterwards the Stop button is disabled with the same sentence and the tray item reads the same.
*Enforced:* `commands/node.rs::stop_hsd`, `commands/local_node.rs::{terminate, OUTSIDE_NODE_NOT_STOPPED}`, `commands/tray.rs::node_menu_texts` (the tray item is disabled with the sentence).
*Pinned:* `node_status_tests::{stopping_an_outside_node_sends_nothing, a_node_that_answers_but_is_not_ours_is_outside}` (mockito `.expect(0)` on the `stop` call), `node_lifecycle_tests::stop_ends_an_own_node_left_by_an_earlier_session`, `tray::tests::an_outside_node_item_is_disabled_with_the_reason`.
*Enforced (UI):* `src/components/Settings.tsx` (the Stop button, `node-stop-button`, disabled with the sentence in a Tooltip) and `src/components/ui/StatusStrip.tsx` (the Stop item disabled), both from `node_status`'s `ownership`; the sentence is `src/lib/nodeNetwork.ts::OUTSIDE_NODE_NOT_STOPPED`.
*Pinned (UI):* `settings-network-nodes.test.tsx`, `node-status.test.tsx`.

### 3.4 Controls

**R8 — Start and Stop act on the active wallet's network and name it.** Settings, the status strip and the tray offer one Start/Stop, for the active profile's network: "Start mainnet node", "Stop regtest node" (`networkDisplayName` / `Network::display_name` spell mainnet as a person reads it). Other networks' nodes are not listed or touched; to stop the mainnet node the user switches to a mainnet wallet. With no active profile there is no network: `start_hsd` refuses with its sentence, `stop_hsd` does nothing, and the tray reads "Start Node".
*Enforced:* `commands/node.rs::{start_hsd, stop_hsd}`, `commands/tray.rs::node_menu_texts`, `src/components/Settings.tsx`, `src/components/ui/StatusStrip.tsx`.
*Pinned:* `node_lifecycle_tests::two_networks_run_side_by_side`, `tray::tests::the_node_item_names_the_active_network`, `settings-network-nodes.test.tsx`, `node-status.test.tsx`.

**R9 — Quit keeps or stops every own node by one rule.** At quit, with background sync on, every own node is left running for the daemon, which syncs every profile, and keeps its pid file, so the next session finds it own (R6). With it off, every own node is stopped: this session's children of every network, and the nodes earlier sessions left, found by their pid files (SIGTERM), each record removed. Outside nodes are left alone either way (R7). Quitting is never blocked by a node.
*Enforced:* `commands/node.rs::release_own_nodes_on_quit`, called from the `RunEvent::Exit` handler in `lib.rs`.
*Pinned:* `node_lifecycle_tests::{quit_with_background_sync_leaves_every_own_node, quit_without_background_sync_stops_every_own_node}`.

**R10 — Mode and autostart are settings of the network.** Settings shows them for the active wallet's network: the full/SPV part of the connection-mode selector and "Start the <network> node automatically". Another network's are set by switching to one of its wallets. Every reader of the mode takes the network's: the spawn flags (`--spv`, or `--index-address --index-tx`) in `start_hsd`, the chain source derived from `chain_source` plus mode (`ChainSource::from_settings`: `local_node` + `spv` is `SpvNode`), and the SPV branches of sync, send and chain scan. They read it through one resolver of the profile's network (`resolve_node_mode_for`), and autostart at app start through `autostart_hsd_for`. The keys are `node_mode.<network>` (`main`, `testnet`, `regtest`, `simnet`, as `Network::as_str` spells them) and `autostart_hsd.<network>` in `settings`. A network with no key of its own reads the app-wide `node_mode` / `autostart_hsd`, so nobody's node changes on upgrade and no migration is needed; until the Settings block of step 7 writes the network keys, the connection-mode selector keeps writing the app-wide ones and every network follows them. Only a caller with no profile, and so no network, reads the app-wide mode alone (`NodeRpcClient::from_settings`, the no-profile arms of `get_write_capability`). SPV keeps its chain in `spvchain/` and a full node in `chain/` (`blockchain/chain.js:4012`), so changing a network's mode never overwrites the other chain.
*Enforced:* `noncustodial/rpc.rs::{resolve_node_mode_for, autostart_hsd_for}`, `noncustodial/node_config.rs::effective_node_config_for_profile`, and the readers in `commands/node.rs::{start_hsd, node_status}`, `commands/node_readiness.rs::is_node_ready_for_local_reads`, `commands/sync.rs`, `commands/tx.rs::get_write_capability`, `commands/chain_scan.rs`, `lib.rs` (autostart at app start).
*Pinned:* `rpc::tests::{a_networks_own_mode_wins_over_the_app_wide_one, a_networks_own_autostart_wins_over_the_app_wide_one}`, `node_config_tests::a_profiles_chain_source_follows_its_networks_mode`, `node_lifecycle_tests::the_networks_mode_picks_the_spawn_flags`, `tx_lifecycle_tests::only_the_wallets_own_networks_spv_mode_blocks_writes`.
*Enforced (UI):* `src/components/Settings.tsx` shows the active wallet's network's keys (else the app-wide ones) in the connection-mode selector's full/SPV part and the autostart toggle ("Start the mainnet node automatically"), and Save writes them to `node_mode.<network>` / `autostart_hsd.<network>` instead of the app-wide keys; `src/lib/nodeNetwork.ts::networkNodeSettingKey` spells the key. The wallet's network can load after the settings; only these two fields follow it then, so no edit is reset.
*Pinned (UI):* `settings-network-nodes.test.tsx`.

### 3.5 What the user sees

**R11 — The node is shown starting.** Between the spawn and the first RPC answer of an own node, `node_status` reports `starting`, and the status strip and Settings read "Starting <network> node…" instead of "Offline" / "Stopped". The other states (syncing, network mismatch, unreachable) are unchanged.
*Enforced:* `commands/node.rs::node_status` (`starting`, `ownership`), `src/components/ui/StatusStrip.tsx`, `src/components/Settings.tsx`.
*Pinned:* `node_status_tests::an_own_node_not_answering_yet_is_starting`, `node-status.test.tsx`, `settings-network-nodes.test.tsx`.

**R12 — The daemon never starts or stops hsd.** `namehold-syncd` reads the nodes that run; a profile whose node does not answer is skipped until the next cycle, as before. SECURITY.md's statement of what the daemon does stays true.
*Enforced:* the daemon module names no node start or stop.
*Pinned:* `shakedex_layering_tests::the_daemon_never_starts_or_stops_hsd` (a source check over every Rust file of the daemon module for `start_hsd`, `stop_hsd`, `local_node::terminate`, `find_hsd_binary`, `hsd_children`).

## 4. Explicitly not enforced

- **No idle stop.** A node whose wallets have not been active for hours keeps running until quit.
- **No list of all running nodes.** Only the active network's node has controls; a running node of another network is visible only by switching to one of its wallets.
- **Not a supervisor.** A node that crashes is not restarted on its own; the next activation of one of its wallets, or Start, starts it again.
- **The daemon does not bring nodes back after a reboot.** Background sync with the app closed works only for nodes still running.

## 5. Known gaps

- **Two full nodes cost memory and CPU.** Accepted: regtest and simnet are cheap, and the costly mainnet node is one the user already ran.
- **A custom global URL serves every local-node profile alike.** A global URL that is not a default is kept for every network's local-node profiles, so two networks pointed at one custom global URL still reach one node; the network guard refuses the wrong chain, and a per-profile override is the way to give each its own. Accepted: it is an explicit setting, and replacing it would reroute a user who configured a node by hand.

- **No own-node record on Windows.** The start time is read on macOS and Linux only, so on Windows a node left running for the daemon counts as outside after a restart and cannot be stopped from the app. Accepted for now: reading it needs `GetProcessTimes` and a Windows test run.

## 6. Implementation plan

Each step is one PR-sized change with its tests, and leaves the app working on its own.

| Step | Change | Requirements | Status |
|------|--------|--------------|--------|
| 1 | A `local_node` profile without an override reaches its network's port over a default global URL; a custom global URL is kept; `start_hsd` stops writing `node_rpc_url` and the realign is removed | R1, R2 | Done |
| 2 | Mode and autostart per network: one resolver of the profile's network with an app-wide fallback (no migration), every reader of the mode switched to it | R10 (backend) | Done |
| 3 | Pid file and own-node recognition by pid and process start time (macOS, Linux) | R6 | Done |
| 4 | One own-node handle per network in `AppState`; Start/Stop per network; an outside node is never stopped | R3, R7 | Done |
| 5 | Start on wallet activation and at app start; switching stops nothing | R4, R5 | Done |
| 6 | Quit by one rule for every own node; source check that the daemon names no hsd spawn | R9, R12 | Done |
| 7 | UI: per-network node settings in Settings, Start/Stop and tray labels naming the network, "Starting…" in the status strip and Settings | R8, R10 (UI), R11 | Done |
| 8 | Live regtest + testnet side-by-side test, `docs/NODE_SETUP.md`, CHANGELOG | R3 (live), docs | Not started |

## 7. Pointers

- Code: `commands/node.rs`, `commands/secure_wallet.rs::activate_profile`, `noncustodial/node_config.rs`, `lib.rs` (startup, tray, exit), `src/components/Settings.tsx`, `src/components/ui/StatusStrip.tsx`.
- Tests: `node_lifecycle_tests`, `node_cmd_tests`, `node_config_tests`, `node_status_tests`, `migration_tests`, `live_node_it`, `settings-network-nodes.test.tsx`, `status-strip.test.tsx`.
- Docs: `docs/NODE_SETUP.md` (one node per network, outside nodes), `CONTEXT.md` (terms above), CHANGELOG `[Unreleased]`.
