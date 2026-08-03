# hsrd sidecar setup

Namehold is the wallet and signer. The `hsrd` process is an authenticated chain
sidecar: it restores script history and UTXOs, supplies name proofs and mempool
evidence, quotes final transaction fees, admits signed transactions, relays them
to peers, and tracks registered swap contracts. Seed material and private keys
never cross this process boundary.

## Build hsrd

Build a current release from the canonical Rust node repository:

```sh
git clone https://github.com/handshake-rs/hns-node-rs.git
cd hns-node-rs
cargo build --release -p hns-node --bin hsrd
install -m 0755 target/release/hsrd ~/.local/bin/hsrd
```

Namehold requires `hsrd` 0.3.4 or newer. You may instead select an explicit
binary under Settings → Connections.

## Managed sidecar

The default managed mode creates a private exact-Authorization file inside the
configured data directory, stores only that exact header value as a redacted
backend setting, and starts `hsrd` on loopback with:

```sh
hsrd --network mainnet --data-dir ~/.hsrd \
  --rpc-bind 127.0.0.1:12037 \
  --rpc-authorization-header-file ~/.hsrd/namehold-wallet.authorization \
  --native-sync --p2p-discovery --wallet-index --storage-mode archive \
  --mining-engine --transaction-relay \
  --acknowledge-incomplete-consensus
```

The `--wallet-index`, `--native-sync`, and authorization-file options are
mandatory for `POST /api/v1/wallet`. The current node release also requires the
explicit incomplete-consensus acknowledgement before transaction relay is
enabled; review the node project's readiness documentation before mainnet use.

Use Start/Stop in Settings to manage the process. Re-sync moves the old data
directory to a timestamped backup before creating a new one, so it is
recoverable and the wallet-index profile is present from the first block.

## Remote sidecar

Set the chain source to `remote_sidecar`, configure the base URL, and enter the
exact Authorization header value expected by the server, for example
`Bearer <token>`. Remote authenticated endpoints must use HTTPS. Plain HTTP is
accepted only for `localhost`, `127.0.0.1`, or `::1`.

The server must expose authenticated wallet RPC v1 at `/api/v1/wallet`.
Namehold rejects unknown envelope fields, mismatched request IDs/API versions,
stale chain epochs, changed mempool generations, malformed canonical
transactions, and invalid name-proof bindings.

## Custody boundary

- Namehold encrypts and stores the seed, performs BIP39/BIP44 derivation, builds
  transactions with hns-rs, and signs locally.
- hsrd sees public scripts, outpoints, canonical signed transaction bytes, and
  opaque tracked-contract IDs. It never receives a seed, extended private key,
  child private key, or signing request.
- The background `namehold-syncd` process performs the same read-only wallet RPC
  restoration as the app and never signs or broadcasts.
