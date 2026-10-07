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

### Name sales (Shakedex)

**Listing**:
A Handshake name its owner has put up for sale through Shakedex: the name sits in the seller's lock, and the seller has signed one or more prices at which anyone may take it. Either a Buy Now or a Reverse auction.
_Avoid_: Offer, paid swap, sale, auction (on its own — see Reverse auction)

**Pending listing**:
An announcement on a market that a name is on its way to being listed: the name is being transferred into its lock and no price is signed yet, so it cannot be bought.
_Avoid_: Draft listing, listing (a listing has signed prices)

**Buy Now**:
A listing offered at a single price, takeable as soon as it is published. A lower price adds a cheaper step to it.
_Avoid_: Fixed proof, fixed-price auction

**Reverse auction**:
A listing whose price steps fall over time; a buyer takes whichever step is currently valid. Not related to a name auction (OPEN → BID → REVEAL).
_Avoid_: Dutch auction (same mechanism; "reverse auction" is what the Shakedex ecosystem shows), auction

**Price step**:
One price the seller has signed for a listing, with the time from which it is valid. A buyer pays exactly that price.
**Current step**:
The cheapest price step of a listing that is valid for the next block. It is what this wallet buys at and what it shows as the price.
_Avoid_: Bid (in this wallet a bid is a buyer's sealed offer in a name auction — the opposite direction), presign

**Listing file**:
The portable form of a listing that buyers, markets and other wallets exchange — the Shakedex proof format, version 2.
_Avoid_: Proof, auction file, swap proof

**Lock**:
The state of a name transferred to the seller's own lock address so that it can be sold through a listing. Leaving it again needs the seller's cancel or a buyer's purchase.
_Avoid_: Escrow (nobody else holds it)

**Lock key**:
The key that controls a name's lock: it alone can cancel the listing or sign its price steps. Each name has its own, derived from the recovery phrase and the name, so listing the same name again reuses it. The wallet never sends coins to it and uses it for nothing else.
_Avoid_: Listing key, swap key

**Purchase**:
A buyer taking one price step of a listing: one transaction that pays the seller and moves the name toward the buyer. The name becomes the buyer's only after it is finalized.
_Avoid_: Fill (Shakedex's word for the transaction, not for what the buyer does), swap, trade

**Awaiting finalize**:
The state of a purchased name between the purchase and its finalize: the price is paid and the name is committed to the buyer, but it still sits at the seller's lock until the transfer lockup has passed and someone finalizes it.
_Avoid_: Pending, incoming transfer (that is a name sent to one of the wallet's own addresses)

**Unconfirmed purchase**:
A purchase sent to the network but not yet in a block. It becomes awaiting finalize, or it is lost — another buyer or the seller's cancel got there first, or it was never mined — and then nothing was paid.
_Avoid_: Pending purchase, failed purchase

**Market fee**:
An optional payment a listing asks the buyer to make to the market that published it, on top of the price. Nobody signs it — whoever passes the listing on can change it — so paying it is the buyer's choice.
_Avoid_: Commission, network fee (that is the miners' fee for the transaction)

**Sold**:
A listing whose lock was bought: the lock coin was spent by a purchase that paid the seller. Money arriving at the payment address on its own is not a sale.
_Avoid_: Filled, completed

**Cancel**:
The seller taking a locked name back: the lock key moves the name to one of the seller's own addresses, which kills every price step of the listing once it is mined. Until then a buyer can still take the current price.
_Avoid_: Delist (taking a listing off a market does not stop anyone holding the listing file from buying), withdraw

**Lower price**:
Signing a new, cheaper price step on an existing listing. Instant and needs no cancel, because any buyer can take the cheapest valid step and this wallet always does. Raising a price is impossible without a cancel.
_Avoid_: Edit price, reprice
