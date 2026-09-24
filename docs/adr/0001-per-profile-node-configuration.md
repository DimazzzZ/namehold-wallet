# Per-profile node configuration with global fallback

When a wallet profile is created for a specific network (mainnet, testnet, regtest), the user may want to point that profile at a different node than the global default — for example, a regtest profile should use a local regtest node, while a mainnet profile uses a remote node. Today, the app has only global `node_rpc_url` and `node_rpc_api_key` settings; all profiles share them, which forces a choice: either all profiles see the same node, or the user manually edits settings each time they switch profiles.

We will store per-profile node configuration in a new `profile_settings` table (key/value pairs per profile_id), with explicit keys `node_rpc_url`, `node_rpc_api_key`, and `chain_source`. Resolution order is: profile override → global setting → built-in default. This keeps the global settings as a fallback and avoids breaking existing code that reads from global settings only.

The resolution function `effective_node_config_for_profile(profile_id)` is called at the start of any operation that needs to know which node to use (sync, read, broadcast, chain scan). It returns `(node_rpc_url, node_rpc_api_key, chain_source)` or an error if the profile does not exist. The function is network-aware: it does not override the profile's network, only the node endpoint. A per-profile override that points to an unreachable or mismatched-network node is treated as a configuration error (not a fallback to global), consistent with the guard in `commands/node_readiness.rs` (`node_tip_height_if_synced_with_client`) and the principle stated in ADR-002 (network-derived behaviour): explicit choices are not silently abandoned.

The frontend will offer a "Configure node for this profile" button in Settings when viewing a profile, and a modal to set/clear the per-profile override. The global Settings → Connections remains unchanged for users who do not need per-profile overrides.

## Consequences

- **Schema change**: new `profile_settings(profile_id, key, value)` table with FK to `wallet_profiles`.
- **Resolution order**: every call site that currently reads `node_rpc_url` from global settings must be updated to call `effective_node_config_for_profile` instead (or a thin wrapper). This is a mechanical change but touches many sites (sync, read, tx, chain_scan, daemon).
- **Error handling**: a per-profile override that fails is not a fallback — it is a user-facing error. The preflight banner will surface this.
- **Backwards compatibility**: existing profiles with no per-profile override continue to use global settings, so no data migration is needed.
- **Test coverage**: new tests for the resolution order, for profile-not-found errors, and for the interaction with the network guard.

## Related

- [ADR-002 Network-derived behaviour](./2026-09-14-network-derived-behaviour.md) — network is immutable per profile; node resolution respects that.
- [Spec: Remote-node connection & broadcast guard](../specs/2026-09-11-remote-node-connection-and-broadcast-guard.md) — R7 already gates the probe on `expected_network`.
- [Spec: Per-profile node banner & preflight](../specs/2026-09-15-per-profile-node-banner-and-preflight.md) — UX/behaviour verdicts from the 2026-09-15 prototype grill, and the step-by-step implementation plan for shipping this ADR.


## Interaction with N11 (`realign_loopback_rpc_url`)

The narrow auto-realign introduced by [ADR-002 / N11](../specs/2026-09-14-network-derived-behaviour.md) rewrites a stale loopback `node_rpc_url` at `start_hsd` time when the stored URL is a *default* loopback port for a *different* network. Per-profile override takes precedence: when a profile has an override set, `start_hsd` and every read/broadcast path resolves through `effective_node_config_for_profile` first, and realign is skipped for that profile. Realign continues to apply only when the resolved node config comes from the global fallback (no per-profile override), preserving the original safety net for users who never opted into per-profile config.

Rationale: an override is an explicit user choice; silently rewriting it — even for a "stale default" — would repeat exactly the failure mode ADR-002 refuses in its "Explicitly not enforced" section (never silently abandon an explicit choice).
