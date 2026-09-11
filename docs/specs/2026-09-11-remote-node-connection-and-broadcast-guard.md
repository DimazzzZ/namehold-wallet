# Remote-node connection & broadcast guard

**Status:** implemented on `feat/spv-broadcast-guard-and-remote-node-onboarding`.
**CHANGELOG:** `## [Unreleased]` → "Remote-node onboarding" (Added), "SPV mode
can no longer broadcast" and "`allow_remote_broadcast` is now enforced" (Fixed).

## 1. Summary

A first-run user chooses how the wallet reaches the Handshake chain — a local
full node, a local SPV node, or someone else's hsd over RPC — before creating
a wallet, and can validate a remote RPC with a "Test connection" probe. The
same chooser and probe exist in Settings → Connections. Broadcasting is
refused at the RPC boundary for read-only sources (SPV, explorer) and for a
remote node unless the user opted in. The probe compares the node's network
with the wallet's once a wallet exists and warns on a mismatch.

## 2. Terms

- **Chain source** (`chain_source` setting): `local_node` | `remote_node` |
  `explorer`. Where the engine reads chain state and broadcasts.
- **Node mode** (`node_mode` setting): `full` | `spv`. Only meaningful for
  `local_node`.
- **ConnectionMode** (frontend, `src/lib/connectionMode.ts`): the four
  user-facing choices `local_full` | `local_spv` | `remote_node` |
  `explorer`, mapped to/from the (`chain_source`, `node_mode`) pair. SPV is
  **not** a `chain_source` value; the backend derives `ChainSource::SpvNode`
  from (`local_node`, `spv`).
- **Probe**: the `check_node_connection` Tauri command. Reads
  `getblockchaininfo` from a candidate URL and persists nothing.
- **Network mismatch**: the node's `chain` and the active wallet profile's
  `network` name different Handshake networks after `main` ↔ `mainnet`
  normalization.
- **Read gate**: `commands::read::node_tip_height_if_synced_with_client` —
  decides whether the local/remote node is authoritative for reads.
- **Broadcast boundary**: `commands::tx::broadcast_tx_draft` and
  `ChainSource::can_broadcast()` in `noncustodial::rpc`.

## 3. Requirements

**R1 — Onboarding opens with the connection choice.** `Onboarding.tsx` shows
"How do you want to connect?" with three cards: Local Full Node (marked
"(default)"), Remote node, SPV. Choosing persists `chain_source` + `node_mode`
(and `node_rpc_url` / API key for remote) via `update_setting` before the
wallet step. Pinned by `src/components/__tests__/onboarding-connection.test.tsx`.

**R2 — Onboarding shows only when no wallet profile exists.** `App.tsx`
renders `<Onboarding />` iff `!onboardingComplete && !hasProfile`.
Consequence: during onboarding there is never an active profile (see R7, G1).

**R3 — Settings offers the same four modes in one selector.** The "Chain
source" `<select>` in `Settings.tsx` uses `toConnectionMode` /
`fromConnectionMode`; the old separate "Node mode" dropdown is gone. Remote
and explorer always write `node_mode: "full"` so a stale `spv` cannot turn a
remote node read-only. Pinned by `src/lib/__tests__/connectionMode.test.ts`
and `src/components/__tests__/settings-test-connection.test.tsx`.

**R4 — The probe persists nothing and honours the transport guard.**
`check_node_connection(url, api_key?)` builds `NodeRpcClient::try_new(url,
key, ChainSource::RemoteNode)`; the plaintext-key / non-loopback guard there
turns an `http://` non-loopback URL with a key into a configuration error
returned to the UI ("fix your URL/key first"), not a connectivity error.
Nothing is written to settings. Backend tests:
`check_node_connection_reports_reachable_and_synced`,
`check_node_connection_reports_not_synced_when_behind_headers`,
`check_node_connection_treats_low_progress_as_not_synced`,
`check_node_connection_surfaces_rpc_error` in
`src-tauri/src/tests/node_rpc_injected_tests.rs`. Frontend: "surfaces a thrown
backend error (e.g. the plaintext-key guard)" in
`src/hooks/__tests__/useNodeConnectionCheck.test.ts`.

**R5 — Stored API key reuse.** `resolve_probe_api_key(url, explicit, settings)`:
an explicit non-empty key wins; otherwise the stored key is used **only** when
the probed URL, trimmed and stripped of a trailing `/`, equals the saved
`node_rpc_url`; otherwise the probe runs with no key. A freshly typed URL
never receives the stored secret. Pinned by
`probe_key_explicit_key_wins_over_stored`,
`probe_key_reuses_stored_key_only_for_the_saved_url`,
`probe_key_empty_when_nothing_stored` in
`src-tauri/src/tests/node_rpc_injected_tests.rs`. Documented in
`docs/NODE_SETUP.md` ("Stored API key").

**R6 — "Synced" is conservative for a first-contact probe.** `synced =
info.is_synced(assume_when_unknown = false)`: a node with no sync metadata is
reported as "syncing", not "synced". (The read gate uses `true` for the same
call because a regtest miner never reports progress — the two call sites are
deliberately different and each says why.)

**R7 — Network comparison in the probe.** `check_node_connection` reads
`db::queries::get_active_profile_network(&conn)` (a DB error is returned to
the UI, not swallowed) and passes it to `check_node_connection_with_client`,
which sets `network_matches = noncustodial::network::network_check(expected,
info.chain)`. Semantics: `Some(false)` = mismatch; `Some(true)` = match;
`None` = nothing to compare (no active profile, or node reports no `chain`).
Pinned by `check_node_connection_flags_a_cross_network_node`,
`check_node_connection_matches_network_across_spellings` and
`check_node_connection_skips_network_check_when_node_reports_no_chain` in
`src-tauri/src/tests/node_rpc_injected_tests.rs`, and by
`network_check_compares_only_when_both_sides_are_known` in
`src-tauri/src/tests/network_tests.rs`.

**R8 — The read gate uses the same rule.** `node_tip_height_if_synced_with_client`
returns `None` (node not authoritative) when `network_check(...) == Some(false)`;
`None` from the check does not reject. Pinned by
`synced_with_matching_network_returns_height`,
`synced_with_mismatched_network_returns_none`,
`synced_with_no_expected_network_skips_check` in
`src-tauri/src/tests/node_status_tests.rs`.

**R9 — The UI treats a mismatched node as not usable.**
`useNodeConnectionCheck().ok === reachable && networkMatches !== false`;
`null` does not block. `ConnectionCheckStatus` shows the green "✓ Connected"
line and, when `networkMatches === false`, an amber line with test id
`connection-network-mismatch` whose text says the app will not read from this
node. Onboarding's Continue is `disabled={!probe.ok}`. Pinned by "ok is false
for a reachable node on a different network than the wallet", "ok stays true
when networkMatches is null (nothing to compare)" (hook tests), "flags a
cross-network node next to the green Connected line" (Settings test), "a
reachable node on the wrong network keeps Continue disabled" (onboarding test —
a contract test, see G1).

**R10 — SPV can never broadcast.** `ChainSource::SpvNode.can_broadcast() ==
false`; `broadcast_tx_draft` refuses up-front with "chain source is read-only;
broadcasting is disabled" and leaves the draft untouched (neither `failed` nor
`broadcast_pending`). Pinned by `broadcast_refused_in_spv_mode` in
`src-tauri/src/tests/tx_lifecycle_tests.rs`.

**R11 — Remote broadcast needs the opt-in.** With `chain_source =
remote_node`, `broadcast_tx_draft` refuses unless
`remote_broadcast_allowed(&settings)` (setting `allow_remote_broadcast ==
"true"`, default off). The toggle "Allow sending via remote node" appears on
the onboarding Remote step and in Settings → Connections. Pinned by
`broadcast_refused_for_remote_node_without_opt_in` in
`src-tauri/src/tests/tx_lifecycle_tests.rs`.

**R12 — TS/Rust lockstep.** `NodeConnectionCheck` in `src/types/index.ts`
mirrors the Rust struct field-for-field (`camelCase` via serde). Every
frontend fixture that builds a `NodeConnectionCheck` includes `networkMatches`.

## 4. Explicitly not enforced

- **Sends are not network-gated by the app.** `broadcast_tx_draft` does not
  compare node chain to profile. A cross-chain transaction is rejected by the
  node (its inputs do not exist there). UI copy and docs must not say "sends
  will be refused". (Review finding on `b49415c`, closed 2026-09-11.)
- **`chain_source` does not route reads.** Reads come from the node when it
  is synced and on the right network (R8), else from the explorer, regardless
  of the selector. The "Read-only (never send)" label describes send
  capability only. Exception: with `node_mode = spv` the node is never
  authoritative for reads (`is_node_ready_for_local_reads` returns false), so
  SPV reads always come from the explorer.
- **The read gate does not fail closed on a DB error.** A failure to load the
  profile network degrades to "no network to compare" (routing decision, not
  a security boundary). The probe (R7) does fail loudly.

## 5. Known gaps

- **G1 — Onboarding cannot detect a mismatch.** Because of R2 there is no
  profile during onboarding, so `network_matches` is always `None` there; a
  user can validate a testnet node, then create a mainnet wallet, and only
  learn of the mismatch when they next press "Test connection" in Settings
  (or when reads route to the explorer). Accepted for now: the wallet's
  network is chosen after the connection step. A future fix would re-probe
  after profile creation or ask for the network first.
- **G2 — Unknown network strings.** `network_name_matches` compares unknown
  strings literally; two different unknown strings are a mismatch, the same
  unknown string on both sides is a match. hsd only ever reports the four
  known names, so this is defensive, not user-visible.

## 6. Pointers

- Backend: `src-tauri/src/commands/node.rs` (probe, `NodeConnectionCheck`),
  `src-tauri/src/commands/read.rs` (read gate),
  `src-tauri/src/commands/tx.rs` (`broadcast_tx_draft`),
  `src-tauri/src/noncustodial/rpc.rs` (`ChainSource`, `can_broadcast`,
  `remote_broadcast_allowed`), `src-tauri/src/noncustodial/network.rs`
  (`network_check`, `network_name_matches`), `src-tauri/src/db/queries.rs`
  (`get_active_profile_network`), `src-tauri/src/commands/active_profile.rs`.
- Frontend: `src/components/Onboarding.tsx`, `src/components/Settings.tsx`,
  `src/components/ui/RemoteNodeFields.tsx`,
  `src/components/ui/ConnectionCheckStatus.tsx`,
  `src/hooks/useNodeConnectionCheck.ts`, `src/lib/connectionMode.ts`,
  `src/types/index.ts`.
- Docs: `docs/NODE_SETUP.md` (Remote node section), `CHANGELOG.md`.
