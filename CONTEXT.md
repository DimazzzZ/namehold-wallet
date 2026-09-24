# Namehold Wallet Domain

The Namehold wallet manages Handshake names and coins on a specific blockchain network. A user creates one or more profiles, each tied to a single immutable network (mainnet, testnet, or regtest), and the wallet reads and broadcasts through a node on that network.

## Language

**Profile**:
A wallet profile is a named container for keys, addresses, and transaction history on one specific Handshake network. Each profile is tied to exactly one network at creation time; the network cannot be changed. A user may create multiple profiles to manage names on different networks (e.g., one mainnet profile and one regtest profile for testing).
_Avoid_: Account, wallet (the app is the wallet; a profile is inside it)

**Network**:
One of `mainnet`, `testnet`, or `regtest`. Immutable per profile. Determines consensus parameters (block time, name auction windows, RPC port defaults, explorer URL).
_Avoid_: Chain (chain is what the node reports; network is what the profile declares)

**Node**:
An hsd instance (full node, SPV, or remote) that the wallet queries for chain state and broadcasts transactions to. A node has a chain (what it reports via `getblockchaininfo`); a profile has a network. They must match for the node to be authoritative.
_Avoid_: Server, RPC endpoint (those are implementation details)

**Chain source** (`chain_source` setting):
Where the wallet reads chain state and broadcasts: `local_node`, `remote_node`, or `explorer`. Only `local_node` can be paired with a node mode (full or SPV).
_Avoid_: Connection mode (that's frontend terminology)

**Node mode** (`node_mode` setting):
For a `local_node` chain source, whether it runs as a full node (`full`) or SPV node (`spv`). Meaningless for remote or explorer sources.
_Avoid_: Sync mode

**Node configuration**:
The tuple `(node_rpc_url, node_rpc_api_key, chain_source)` that tells the wallet how to reach a node. Can be global (applies to all profiles) or per-profile (applies only to that profile). Per-profile overrides the global.
_Avoid_: Connection settings (too vague)

**Per-profile override**:
A per-profile choice of node configuration that takes precedence over the global default. Stored in the `profile_settings` table, one row per key. If an override is set and invalid (unreachable, mismatched network), it is a configuration error for that profile, not a fallback to global.
_Planned, not built_: splitting this per *slot*, so a profile could override its read slot (e.g. read via explorer) while inheriting the write slot from global. The table has no slot column and resolution returns one tuple; see step 8 of the [per-profile node spec](./docs/specs/2026-09-15-per-profile-node-banner-and-preflight.md). Until then "per-slot per-profile override" names something the wallet does not do.
_Avoid_: Profile-specific node, profile node setting

**Preflight**:
A check performed before an operation (sync, read, broadcast) to ensure the node is reachable and on the correct network for the active profile. Returns a status (ready, missing, misconfigured) and optionally a suggested fix.
_Avoid_: Health check (preflight is narrower; it's about readiness for a specific profile)

**Network mismatch**:
The node's reported chain (from `getblockchaininfo`) does not match the profile's network after normalization (`mainnet` ↔ `main`). A positive mismatch is a configuration error and blocks operations.
_Avoid_: Chain mismatch (same thing, but "network" is the profile's term)

**Effective node config**:
The resolved node configuration for a profile after applying the resolution order, evaluated independently per key: per-profile override (if set and non-empty) → global settings (if set) → built-in default. The result is one `(node_rpc_url, node_rpc_api_key, chain_source)` tuple, plus whether the profile's own override supplied it — which is what keeps the loopback-port realign off a URL the user chose. A profile that does not resolve is an error, never a fallback to global.
_Avoid_: Resolved config (same meaning, but "effective" emphasizes the resolution order)
