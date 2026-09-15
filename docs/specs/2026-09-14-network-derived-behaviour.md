# Network-derived behaviour

## 1. Summary

Handshake's consensus parameters differ per network, and the wallet had been
treating them as incidental: a value hsd defines per network was either hardcoded
to mainnet or derived from the network in one place and not in the neighbouring
one. Two bugs shipped from that (a reversed renewal-block hash and an unfiltered
immature coinbase), and a regtest end-to-end pass found both.

This spec makes the active profile's network a first-class input. A node on
another chain is refused everywhere rather than only on reads. Coin maturity is
reflected in the balance the user sees, not only in coin selection. The RPC port,
the block-to-days conversions and the explorer follow the profile's network
instead of assuming mainnet.

The user-visible result: the wallet works on regtest and testnet without manual
fixups, and cannot be silently pointed at the wrong chain.

## 2. Terms

- **Profile network** — the `network` column of the active `wallet_profiles` row.
  Immutable after insert: no `SET network` statement exists. One of `mainnet`,
  `testnet`, `regtest` (the SQL `CHECK` and `validate_network` allow no others).
- **Node chain** — the `chain` field of the node's `getblockchaininfo`, reported
  by hsd as `main` / `testnet` / `regtest` / `simnet`.
- **Match** — `network_name_matches` compares the two after canonicalising
  `mainnet` to `main`. `network_check` returns `None` when either side is unknown.
- **Positive mismatch** — `network_check` returns `Some(false)`. Only this
  refuses; unknown never does.
- **Authoritative node** — reachable, fully synced, and not a positive mismatch.
  The condition under which node data outranks the explorer.
- **Spend height** — `tip + 1`, the height a transaction built now would be mined
  at. hsd validates coinbase maturity against this, not against the tip.
- **Mature coinbase** — a coinbase coin with `height + coinbase_maturity <=
  spend_height`. Anything else is **immature** and unspendable.

## 3. Requirements

### Chain identity

**N1 — One gate, no opt-out.** Every "is this node authoritative?" decision
compares the node chain to the profile network. `read.rs` exposes no helper that
skips the comparison: the `State`-based `node_tip_height_if_synced` resolves the
active profile itself, and the settings-based
`node_tip_height_if_synced_from_settings_with_network` and
`node_ready_from_settings` take the expected network as a required argument.
Enforced by the absence of a network-less wrapper; pinned by
`tests/node_status_tests.rs::node_ready_from_settings_false_when_node_is_on_another_chain`.

**N2 — Background workers are gated.** The background sync
(`commands/sync.rs::run_sync_steps`), the chain scanner
(`commands/chain_scan.rs::run_chain_scanner`) and the watched-name daemon
(`daemon/watched_names.rs::try_run_watched_scan`) each resolve a network and pass
it to the gate. The sync uses the network of the profile being synced, not the
active one.

**N3 — Syncing from a foreign chain is refused.**
`commands/tx.rs::sync_wallet_state` compares `info.chain` to the profile network
before any write and returns an error on a positive mismatch. Nothing reaches
`sync_cursors`, `wallet_profiles.last_synced_height` or `tracked_name_states`.
This requirement exists because `sync_cursors` is the tip
`noncustodial/send.rs::load_spendable_coins` reads to decide coinbase maturity: a
foreign height there silently corrupts N6.

**N4 — Broadcasting to a foreign chain is refused.**
`commands/tx.rs::broadcast_network_guard_with_client` probes the node and returns
an error on a positive mismatch, before the signed transaction is handed over. The
draft is left untouched: neither `failed` nor `broadcast_pending`. Pinned by
`tests/node_rpc_injected_tests.rs::broadcast_guard_refuses_a_node_on_another_chain`
and `..._allows_matching_unknown_and_unreachable`.

*This reverses a decision.* The 2026-09-11 remote-node spec listed sends as
explicitly not network-gated, on the reasoning that a cross-chain transaction
spends coins the node has never seen and is rejected anyway. That reasoning fails
open: the rejection arrives transport-shaped often enough to strand the draft in
`broadcast_pending`, and it leaks a signed transaction to a node the user never
meant to talk to. That spec's entry is marked superseded and points here.

**N5 — A foreign chain is never reported as ready to send.**
`commands/tx.rs::apply_node_write_probe_with_client` checks the chain before the
sync and address-index checks, because a regtest node is 100% synced and fully
indexed and would otherwise pass every one of them. The reason names both
networks. `Settings.tsx` refuses to save a node the probe positively identified as
mismatched, matching the gate Onboarding already applied.

### Coin maturity

**N6 — Maturity follows the network and the consensus rule.**
`Network::coinbase_maturity` returns hsd's per-network `coinbaseMaturity`
(mainnet and testnet 100, regtest 2, simnet 6).
`noncustodial/send.rs::load_spendable_coins` excludes a coinbase coin unless
`height + maturity <= tip + 1`, matching
`lib/primitives/tx.js::checkInputs`, which hsd calls with `chain.height + 1`.
Pinned by `load_spendable_coins_excludes_immature_coinbase`,
`..._includes_coinbase_exactly_at_maturity` and
`..._applies_the_networks_own_maturity`.

**N7 — A never-synced wallet treats every coinbase as immature.** With no
`sync_cursors` row the tip reads 0, so the predicate fails for every coinbase
coin. Pinned by `load_spendable_coins_excludes_all_coinbase_when_never_synced`.

**N8 — The balance shown as spendable is spendable.**
`noncustodial/sync.rs::compute_balances` applies the N6 predicate with the same
tip, so `Balances::liquid` counts only what coin selection would select. Immature
coinbase value is reported in `Balances::immature` with
`Balances::immature_in_blocks`, the wait until the earliest coin matures.
`Balances::total` still counts it — the wallet owns it. Pinned by
`compute_balances_splits_immature_coinbase_out_of_liquid`,
`..._uses_the_networks_own_maturity` and
`..._treats_all_coinbase_as_immature_before_a_sync`.

**N9 — The send form offers only mature funds.** `WalletBalances.immatureDoos`
and `immatureInBlocks` cross the bridge; `WalletView` derives `spendable` from
`liquidDoos`, which now excludes immature value, so the Send button, the Max
button and the amount validation all refuse what the backend would refuse. A
non-zero immature balance renders its own cell with the block count. Pinned by
`wallet-view.test.tsx` ("shows freshly mined coins as Immature…" and "hides the
Immature cell…").

**N10 — A shortfall caused by maturity says so.**
`noncustodial/send.rs::shortfall_message` returns a distinct message when
immature value alone would have covered the amount, and `commands/tx.rs::explain_shortfall`
applies it at both build paths. `src/lib/errors.ts` maps it ahead of the generic
"insufficient funds" entry, which `mapError` would otherwise match first. The
message stays generic when maturing would not close the gap — promising a wait
that will not help would be a lie. Pinned by the three `shortfall_message_*` tests.

### Connection defaults

**N11 — The local node's RPC port follows the profile's network.**
`Network::default_rpc_port` carries hsd's per-network `rpcPort` (12037 / 13037 /
14037 / 15037). `start_hsd` launches hsd with the profile's network flag, so hsd
listens on that port; before spawning it now realigns a stale `node_rpc_url` via
`noncustodial/rpc.rs::realign_loopback_rpc_url`. Settings' URL placeholder uses
the TS mirror `defaultNodeRpcUrl`. Pinned by
`test_default_rpc_port_all_variants`, `realign_rewrites_a_stale_default_for_the_target_network`
and `realign_leaves_deliberate_urls_untouched`.

The rewrite is deliberately narrow: it fires only when the stored URL is both
loopback **and** on a recognised default port for a *different* network, which
makes it a leftover seed rather than a choice. A custom port, a remote host, or
a URL already on the right port is never touched.

### Time, thresholds and the explorer

**N12 — The expiry warning scales with the network's renewal window.**
`Network::expiring_soon_threshold_days` returns a fixed share (30/730) of
`name_params().renewal_window`, keeping mainnet's established 30 days while
giving testnet ~1.2, regtest ~1.4 and simnet ~0.7. A flat 30 days is the entire
testnet window and more than the simnet one, so every owned name there sat
permanently in "expiring soon". `derive_auction_task_state` and
`build_name_action_capabilities` take the profile's `Network`;
`compute_renewals` reports the same figure to the frontend as
`expiringSoonThresholdDays`. Pinned by
`test_expiring_soon_threshold_scales_with_the_renewal_window`.

**N13 — A stored height is only aged by wall clock where blocks follow one.**
`Network::has_wall_clock_block_timing` is true for main and testnet only.
`commands/read.rs::estimate_persisted_height` ages its candidates by
`elapsed_seconds / 600` on those networks and by zero elsewhere. Regtest and
simnet mine on demand, so the old arithmetic invented six blocks for every idle
hour and every renewal countdown drifted. A stale height is reported as stale.
Pinned by `test_has_wall_clock_block_timing_all_variants`.

**N14 — Mainnet-only explorer links appear only on mainnet.** `NameInfoModal`
and `NameActionsModal` gate their "View on explorer" link on
`profile.network === "mainnet"`, as `TxInfoModal`, `BlockInfoModal`,
`ReceiveAddressList` and `WalletView` already did. Shakeshift indexes no other
chain, so the link 404s elsewhere.

**N15 — Starting a node refuses rather than guessing mainnet.**
`commands::active_profile::active_profile_network_opt` keeps "no profile"
distinguishable from "mainnet", and `start_hsd` returns an error instead of
launching. Guessing here is an action with consequences: it begins a full
mainnet chain sync in a data dir prepared for another network. The defaulting
form stays for callers that only *label* a network (status payloads, the
mainnet-only Namebase paths). Pinned by
`the_optional_form_reports_no_profile_as_none`.

## 4. Explicitly not enforced

- **The app does not verify the node is honest about its chain.** Every guard
  here trusts `getblockchaininfo.chain`. A malicious node can claim any network.
  These guards prevent misconfiguration, not an adversary.
- **An unknown chain is not a mismatch.** A node that omits `chain` (older hsd
  builds), a profile that could not be loaded, and a failed probe all pass
  through. The guards refuse only a positive mismatch, so none of them can lock a
  user out of their own node over a missing field.
- **The network of an existing profile cannot be changed.** Not a guard, an
  absence: no command and no `UPDATE` writes the column. Changing network means
  creating another profile.
- **`NodeRpcClient::from_settings` still falls back to the mainnet port.** Its
  42 construction sites make threading a network through it a disproportionate
  change, and the fallback only applies when `node_rpc_url` is absent — which
  migration `009` makes impossible in practice. The setting itself is what N11
  keeps correct.
- **The explorer read fallback is network-gated.** ~~Previously a gap.~~
  `providers::explorer_client_from_settings` now takes a `Network` and
  returns `Option<HnsFansClient>`. Resolution order: explicit
  `explorer_api_url` from settings > `Network::default_explorer_base_url`
  (mainnet only) > `None`. On testnet/regtest/simnet with no explicit URL the
  factory returns `None`, and every read/sync call site threads that through
  as either a candid "explorer unavailable" error or a degraded empty result
  — never a silent mainnet query.
- **Notification lead defaults are not scaled per network.**
  `reveal_lead_blocks` (144) and `DEFAULT_BIDDING_SOON_LEAD_BLOCKS` (144) exceed
  the entire reveal and bidding windows on test chains, so those notices are on
  for the whole period. They are stored, user-editable settings, and rewriting a
  stored setting when a different profile becomes active would surprise more
  than a generous default. Documented at both constants instead.
- **`BLOCKS_PER_DAY` stays a single constant.** Every hsd network sets
  `pow.targetSpacing` to 600s, so a per-network accessor would return the same
  144 four times. The real per-network question is whether blocks arrive on that
  schedule at all, which is N13.
- **`Network::Simnet` is unreachable from the app.** The backend implements it
  for parity with hsd, but the TS `WalletNetwork` union, the only network
  `<select>` (`AddWalletForm.tsx`), `validate_network` and the SQL `CHECK` all
  exclude it.

## 5. Known gaps

- The broadcast guard costs one extra `getblockchaininfo` round-trip per
  broadcast. Accepted: it is one local call before moving money.
- Immature coinbase maps to `HsdBalance.unconfirmed` in the cached read model
  (`read_cached_balance`). It is the closest of that shape's four buckets, but
  it is not literally unconfirmed. The mempool split that bucket was named for
  still does not exist.
- `select_coins` itself has no view of the balance split, so the maturity-aware
  message is assembled by its caller. A future caller that forgets
  `explain_shortfall` gets the generic message, not a wrong one.
- `shortfall_message` compares against the requested amount, not amount plus
  fee, because the fee is not known at that point. Right at the boundary it can
  therefore blame maturity for a gap the fee would reopen. The claim it makes
  ("these coins cannot be spent until they mature") stays true either way.
- Which of the two messages appears depends on the block reward, which halves as
  a chain grows. The live test only asserts the build was refused; the wording
  is pinned by unit tests that do not depend on chain state.
- Settings blocks saving only a *tested* mismatched node. A user who never
  presses "Test connection" can still save a wrong-chain URL; the runtime guards
  then refuse it. Accepted rather than forcing a probe before every save.

## 6. Pointers

- Gates: `src-tauri/src/commands/read.rs`, `commands/tx.rs`,
  `commands/sync.rs`, `commands/chain_scan.rs`, `daemon/watched_names.rs`.
- Network parameters: `src-tauri/src/noncustodial/network.rs`.
- Maturity filter: `src-tauri/src/noncustodial/send.rs`.
- Tests: `src-tauri/src/tests/node_status_tests.rs`,
  `tests/node_rpc_injected_tests.rs`, `tests/network_tests.rs`,
  the `#[cfg(test)]` module in `noncustodial/send.rs`.
- Frontend: `src/components/Settings.tsx`, `src/types/index.ts`.
- Docs: `docs/NODE_SETUP.md`, `docs/REGTEST_TESTING.md`.
- Supersedes the "sends are not network-gated" entry in
  [2026-09-11 Remote-node connection & broadcast guard](./2026-09-11-remote-node-connection-and-broadcast-guard.md).
- `CHANGELOG.md` → `## [Unreleased]`.
