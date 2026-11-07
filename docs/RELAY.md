# Running a relay and watchtower (`namehold-relay`)

`namehold-relay` is the server of the HNS ⇄ BTC exchange ([spec](specs/2026-11-07-hns-btc-exchange.md), API in §8). One process does two jobs:

- **Relay.** It carries the order book (signed offers) and the trade messages between two Namehold apps.
- **Watchtower.** It keeps the presigned transactions a party registers after its Commit and sends them at the right moment: the Seller's claim of the BTC lock as soon as the secret appears on the HNS chain, and each party's refund once its time has come. Every minute it runs on every job the same step the app's own daemon runs (`exchange_io::runner::run_step`).

## What it can and cannot do

The server holds **no private key and no secret** that is not already public. It cannot move anyone's coins.

- It stores an offer or a message only after checking its signature (`relay_msg`); it signs nothing.
- A watch job holds only the party's own presigned transactions (`WatchJob`): the Buyer's BTC refunds, or the Seller's HNS refund and BTC claims with an empty secret slot. Each pays the party that signed it. The Buyer's secret and its HNS claim are not part of a job.
- Every app re-checks everything it receives: offers and messages by their signatures, chain facts on chain.

**Worst case**, a dishonest or broken server hides offers or messages, or fails to send a transaction. That costs time, not coins: each party's own daemon sends the same transactions, and the refund paths stay open. The one risk a watchtower reduces is a Seller whose daemon is offline from the reveal until the BTC refund time (spec §3.4).

Limits that follow from this design:

- A signed offer is public. Anyone who has fetched one can post it again, and posting is the heartbeat, so a third party can keep an offer listed until it expires or its maker withdraws it. A withdrawal is final: the server refuses the same offer until it expires.
- A job is identified by `<terms id>-<role>`, and the server keeps the first job registered under an id; a different job under the same id is refused (409). A job is taken only when every presigned spend in it spends the job's lock and carries the lock key's valid signature (`WatchJob::check`, spec R19), so only the holder of that key can register it: a counterparty who knows the terms cannot take the other party's id first.

## Build

```sh
cd src-tauri
cargo build --release --features relay-server --bin namehold-relay
```

The binary is `src-tauri/target/release/namehold-relay`. The `relay-server` feature adds `axum` to the build; without it (the desktop app's build) neither the binary nor `axum` is built.

## Run

```sh
namehold-relay /etc/namehold-relay/config.json
```

It logs to standard error: one line per request (method, path, status, time; never a body or a query string), one line per watch job that sent something or hit an error, and the hourly purge. SIGINT or SIGTERM stops it after the requests under way.

## Configuration

A JSON file. Unknown keys are refused, and everything is checked at start; a server with a bad configuration does not start.

| Key | Meaning |
|---|---|
| `listen` | Address and port, e.g. `127.0.0.1:8787`. |
| `dbPath` | The server's own SQLite database (WAL mode), created if missing. On Unix the file is owner-only (0600), and a directory it has to create is too (0700); an existing directory is left as it is. Not the wallet's `portfolio.db`. |
| `networks` | One entry per served network, named `main`, `testnet`, `regtest` or `simnet`. |
| `networks.<n>.node` | Optional `{ "url": …, "apiKey": … }` of an hsd node on that network, started with `--index-tx --index-address`. Without it the watchtower is off for that network: the relay serves it, and watch jobs of that network are refused. |
| `networks.<n>.btcSources` | Bitcoin sources for the watchtower: Esplora base URLs (`https://mempool.space/api`) or Bitcoin Core (`bitcoind+http://user:pass@host:port`, with `-txindex`). Omitted: two public Esplora servers on `main` and `testnet`, none on `regtest` and `simnet`. A network with a node needs at least one. |

`http://` is accepted only to this machine (`127.0.0.1`, `::1`, `localhost`), for the node and for Bitcoin sources alike; anything else must be `https://`. The server itself speaks plain HTTP, and the app refuses a relay over `http://` that is not on its own machine: to serve others, listen on loopback and put a TLS reverse proxy (Caddy, nginx) in front.

Every round, the watchtower checks that the node reports the job's network. A node that reports another network, or none, is not used for that round: the step then decides nothing on the HNS chain, so in particular no Buyer refund is sent on its word.

The file holds the node's API key and possibly a Bitcoin Core password: keep it readable by the server's user only.

### Regtest

```json
{
  "listen": "127.0.0.1:8787",
  "dbPath": "/var/lib/namehold-relay/regtest.db",
  "networks": {
    "regtest": {
      "node": { "url": "http://127.0.0.1:14037", "apiKey": "test" },
      "btcSources": ["bitcoind+http://user:pass@127.0.0.1:18443"]
    }
  }
}
```

### Mainnet

```json
{
  "listen": "127.0.0.1:8787",
  "dbPath": "/var/lib/namehold-relay/main.db",
  "networks": {
    "main": {
      "node": { "url": "http://127.0.0.1:12037", "apiKey": "<the node's api key>" }
    }
  }
}
```

`btcSources` is omitted, so the watchtower reads Bitcoin from `https://mempool.space/api` and `https://blockstream.info/api`, and accepts a lock only when both report the same block. Serve the relay to the public through a TLS reverse proxy on `https://`.

## Rules the server enforces

| Rule | Answer |
|---|---|
| A body larger than 16 KiB | 413 |
| An offer or message whose signature does not check, an offer that fails `check_shape`, a message of an unknown version, a network the server does not serve | 400 |
| More than 20 live offers for one maker key (live: renewed within 180 s, not expired, not withdrawn) | 429 |
| A withdrawn offer posted again | 410 |
| A withdrawal not signed by the offer's maker key | 403 |
| More than 200 messages from one sender key in any hour | 429 |
| A watch job that fails `WatchJob::check`, or of a network without a watchtower | 400 |
| Another job under an id already registered | 409 |

Posting an offer again is its heartbeat (200); posting the same message or the same job again stores nothing (200). Offers are listed while renewed within the last 180 s and not expired, newest first, at most 500; messages in arrival order, at most 200 per call.

## What it keeps, and for how long

| Data | Kept |
|---|---|
| Offers | until they expire; one not renewed for 7 days is removed; a withdrawn one is kept (unlisted) until it expires, so it cannot be posted again |
| Messages | 7 days |
| Watch jobs | while the trade runs; a job whose status is final (completed, refunded, expired) is re-checked for a day, then kept 30 days for `GET /v1/watch/<id>` |

The purge runs every hour.
