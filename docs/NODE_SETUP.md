# Connect Namehold to hsrd

Namehold keeps and signs with your keys. `hsrd` supplies authenticated chain
state, wallet history and UTXOs, name evidence, mempool reconciliation, fee
quotes, transaction admission/relay, and swap-contract tracking through wallet
RPC v1. Seed material and private keys never cross that process boundary.

The quickest setup is the multi-platform Docker image. The steps below use a
full archive node and work on Linux x86_64 and ARM64. Docker Desktop users can
use the same published-port layout.

## Docker setup

### 1. Create the shared Authorization value

Namehold and hsrd must use the same exact HTTP `Authorization` value. Create it
once in a private file:

```sh
HSRD_AUTH_FILE="$HOME/.hsrd/namehold-wallet.authorization"
install -d -m 0700 "$(dirname "$HSRD_AUTH_FILE")"
if [ ! -s "$HSRD_AUTH_FILE" ]; then
  HSRD_AUTH_TOKEN="$(openssl rand -hex 32)"
  (umask 077; printf 'Bearer %s\n' "$HSRD_AUTH_TOKEN" > "$HSRD_AUTH_FILE")
  unset HSRD_AUTH_TOKEN
fi
chmod 0600 "$HSRD_AUTH_FILE"
```

Do not put this value in a Docker environment variable, image, Compose file,
command-line argument, log, or screenshot. hsrd reads it from the mounted file.

### 2. Start the archive node

Choose a durable directory on a disk with room to grow. There is no artificial
160 GB allocation or Docker size cap: the archive uses its actual chain/index
size and continues growing with the network. hsrd preserves a built-in 10 GB
free-space safety reserve and stops instead of silently pruning.

```sh
HSRD_DATA_DIR="$HOME/.local/share/hsrd-mainnet-archive"
HSRD_AUTH_FILE="$HOME/.hsrd/namehold-wallet.authorization"
install -d -m 0700 "$HSRD_DATA_DIR"

docker pull ghcr.io/handshake-rs/hns-node-rs:canary-v0.3.4

docker run --detach \
  --name namehold-hsrd \
  --restart unless-stopped \
  --stop-timeout 120 \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --security-opt no-new-privileges \
  --cap-drop ALL \
  --user "$(id -u):$(id -g)" \
  --publish 127.0.0.1:12037:12037 \
  --volume "$HSRD_DATA_DIR:/var/lib/hsrd" \
  --volume "$HSRD_AUTH_FILE:/run/hsrd-authorization:ro" \
  ghcr.io/handshake-rs/hns-node-rs:canary-v0.3.4 \
  --network mainnet \
  --data-dir /var/lib/hsrd \
  --rpc-bind 0.0.0.0:12037 \
  --rpc-authorization-header-file /run/hsrd-authorization \
  --native-sync \
  --p2p-discovery \
  --wallet-index \
  --storage-mode archive \
  --mining-engine \
  --transaction-relay \
  --acknowledge-incomplete-consensus
```

Only host loopback publishes the RPC port. Do not change the host side of the
mapping to `0.0.0.0`; the Authorization value grants wallet RPC and broadcast
authority. The node remains outbound-only for P2P unless you deliberately add
and publish a P2P listener.

### 3. Connect Namehold

Open **Settings → Connections** and set:

| Setting | Value |
|---|---|
| **hsrd RPC base URL** | `http://127.0.0.1:12037` |
| **Exact Authorization header** | The complete one-line value in `~/.hsrd/namehold-wallet.authorization` |
| **Autostart HSRD when the app launches** | Off (Docker owns the process lifecycle) |
| **Sync in background** | On if you want Namehold's read-only wallet cache refreshed while the app is closed |

Save the settings. To copy the exact value for the one-time paste, display the
file locally with `cat ~/.hsrd/namehold-wallet.authorization`; do not share the
output. Namehold stores it as a redacted backend setting and never returns it to
the web UI.

Within one status poll, the panel should show:

```text
Syncing · N% (external node)       Managed externally
External authenticated sidecar: http://127.0.0.1:12037
```

`Local cache / auxiliary provider` and unavailable sending are expected until
the first full sync reaches 100%. Then the authenticated sidecar becomes the
authoritative read source and spend/name actions can use its wallet index.

### 4. Check synchronization

The compact endpoint has the fields most operators need. This filter removes
the peer-detail noise:

```sh
HSRD_AUTH_VALUE="$(tr -d '\r\n' < ~/.hsrd/namehold-wallet.authorization)"

curl -fsS -H "Authorization: $HSRD_AUTH_VALUE" \
  http://127.0.0.1:12037/api/v1/sync |
jq '{
  stage,
  active: .active_tip.height,
  stored: .stored_tip.height,
  headers: .best_header.height,
  target: .target_height,
  percent: ((.active_tip.height * 10000 / .target_height | floor) / 100),
  pending: .pending_blocks,
  inflight: .inflight_blocks,
  validated: .validated_blocks,
  failed: .failed_blocks,
  peers: (.peers | length)
}'

unset HSRD_AUTH_VALUE
```

`active` is the fully validated and connected height. `stored` can be ahead
because hsrd downloads and validates blocks before committing them to active
state. Initial synchronization is complete when `active` reaches `target`.

For deeper runtime diagnostics:

```sh
HSRD_AUTH_VALUE="$(tr -d '\r\n' < ~/.hsrd/namehold-wallet.authorization)"
curl -fsS -H "Authorization: $HSRD_AUTH_VALUE" \
  http://127.0.0.1:12037/api/v1/native-sync | jq
unset HSRD_AUTH_VALUE
```

`/api/v1/native-sync` adds peer traffic, validation queues, active-state timing,
retry counters, and `last_error`. Neither endpoint reports an ETA directly.

### 5. Operate the container

```sh
# Is it running? A restart count of zero confirms it has not restarted.
docker inspect --format 'running={{.State.Running}} restarts={{.RestartCount}}' namehold-hsrd

# Follow node logs.
docker logs --follow namehold-hsrd

# Current archive size.
du -sh "$HOME/.local/share/hsrd-mainnet-archive"

# Stop/start without deleting chain data.
docker stop namehold-hsrd
docker start namehold-hsrd
```

Do not attach two hsrd processes to the same data directory. Removing the
container does not remove this bind-mounted archive directory.

## Native managed sidecar

Docker is optional. To let Namehold start a host-native binary, build hsrd 0.3.4
or newer:

```sh
git clone https://github.com/handshake-rs/hns-node-rs.git
cd hns-node-rs
cargo build --release -p hns-node --bin hsrd
install -m 0755 target/release/hsrd ~/.local/bin/hsrd
```

In **Settings → Connections**, select the binary and archive data directory,
leave autostart enabled, and click **Start hsrd**. Namehold creates the private
Authorization file and starts the same native-sync, wallet-index, archive,
transaction-relay profile shown above. Re-sync moves the old managed data
directory to a timestamped backup before starting clean.

## Remote sidecar

Set the chain source to `remote_sidecar`, configure its base URL, and enter the
exact Authorization value expected by the server. Remote authenticated endpoints
must use HTTPS; plaintext HTTP is accepted only for `localhost`, `127.0.0.1`, or
`::1`.

The server must expose authenticated wallet RPC v1 at `/api/v1/wallet`.
Namehold rejects mismatched request IDs/API versions, stale chain epochs,
changed mempool generations, malformed canonical transactions, and invalid
name-proof bindings.

## Custody boundary

- Namehold encrypts and stores the seed, performs BIP39/BIP44 derivation, builds
  transactions with hns-rs, and signs locally.
- hsrd sees public scripts, outpoints, canonical signed transaction bytes, and
  opaque tracked-contract IDs. It never receives a seed or private key.
- `namehold-syncd` performs read-only wallet restoration and never signs or
  broadcasts.
