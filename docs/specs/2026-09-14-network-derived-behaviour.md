# Network-derived behaviour

## 1. Summary

Handshake's consensus parameters differ per network, and the wallet had been
treating them as incidental: a value hsd defines per network was either hardcoded
to mainnet or derived from the network in one place and not in the neighbouring
one. Two bugs shipped from that (a reversed renewal-block hash and an unfiltered
immature coinbase), and a regtest end-to-end pass found both.

This spec makes the active profile's network a first-class input. A node on
another chain is refused everywhere rather than only on reads. The RPC port, the
block-to-days conversions and the explorer follow the profile's network instead
of assuming mainnet.

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
