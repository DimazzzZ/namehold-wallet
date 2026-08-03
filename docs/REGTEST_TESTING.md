# Testing Namehold with hsrd on regtest

Namehold talks only to authenticated wallet RPC v1. That boundary intentionally
does not expose arbitrary block generation, private keys, or general-purpose
node compatibility methods.

## Start the sidecar

Create a one-line private authorization file, then start the current `hsrd`
binary with its durable wallet index:

```sh
install -d -m 0700 /tmp/namehold-hsrd-regtest
printf 'Bearer namehold-regtest\n' > /tmp/namehold-hsrd-regtest/authorization
chmod 0600 /tmp/namehold-hsrd-regtest/authorization

hsrd --network regtest \
  --data-dir /tmp/namehold-hsrd-regtest/data \
  --rpc-bind 127.0.0.1:14037 \
  --rpc-authorization-header-file /tmp/namehold-hsrd-regtest/authorization \
  --native-sync --p2p-discovery --wallet-index --storage-mode archive \
  --mining-engine --transaction-relay \
  --acknowledge-incomplete-consensus
```

Configure Namehold with:

- RPC URL: `http://127.0.0.1:14037`
- Authorization: `Bearer namehold-regtest`
- chain source: managed or remote sidecar
- wallet network: regtest

Mine or import regtest blocks through the node project's supported mining/P2P
tooling. Namehold does not request blocks or mining through the wallet RPC.

## Required lifecycle checks

After funding a derived receive address, validate:

1. restoration returns confirmed history/UTXOs and reconciles mempool receives
   and spends against one bound snapshot;
2. send performs local signing, obtains a final signed-artifact fee quote, and
   broadcasts only when the sidecar reports the minimum policy fee is met;
3. OPEN → BID → REVEAL → REGISTER and loser REDEEM flows survive confirmation
   and restart restoration;
4. UPDATE, RENEW, TRANSFER, FINALIZE, CANCEL, and REVOKE use canonical hns-rs
   transaction/covenant/script encodings;
5. name state is accepted only after strict Urkel proof verification against
   the returned tip root; and
6. registered swap contracts return bound confirmed and mempool funding/spend
   evidence without exposing redemption preimages.

## Automated checks

```sh
pnpm test -- --run
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
```

Automated integration tests use strict mock wallet-RPC envelopes and need no
process. The live lifecycle above remains a manual system test because wallet
RPC intentionally does not expose mining controls.
