# mine-and-sync

Move the chain and let the app see it.

- **Parameters:** `N` blocks; optional address (default the hsd wallet's `$WADDR`, or the seller's receive address to fund it at the same time).
- **Blocks mined:** `N`.

## Steps

1. `mine N [address]` ([helpers](../environment.md#regtest-node-and-hsd-wallet)), or `scripts/regtest.sh mine N [address]`.
2. In the app, press **Sync** on the Wallet page and wait until it is idle again; or, in a case that tests the background jobs ([J1](../scenarios/shakedex/persistence-and-recovery.md)), do not press it and watch instead.
3. Record the height (`scripts/regtest.sh rpc getblockcount`).

## Lockup arithmetic

The transfer lockup is 10 blocks: a TRANSFER mined at height `h` can be finalized by a transaction in block `h + 10`. So after the `mine 1` that mines a lock or a cancel, the row reads `… 9 blocks`, and `mine 9` more makes it ready.

## Making a price step valid (median time)

Price steps are judged against the chain's median time past (MTP, `getblockchaininfo` → `mediantime`), the median of the last 11 block timestamps (spec R3: a step is valid once `floor(lockTime/512)*512 < MTP`). On regtest MTP moves only when blocks are mined, and never with `setmocktime` here ([rules](../rules.md#the-regtest-chain)).

1. Wait in real time until the step's time has passed (My listings `All N price steps`, or the `in ~T` the row shows).
2. `mine 11`, then Sync.

A step "in ~15 min" therefore costs 11 blocks and about 15 minutes. A Buy Now step signed by the app is valid at once (its lock time is the MTP minus 512 s). A listing file the CLI has just written may read `Not valid yet; the first price step becomes valid in ~T.` for a few minutes (its lock time is the wall clock, the MTP lags): wait the time it names, `mine 11`, and import again.

## End state

The chain is `N` blocks taller; the app has synced to it.
