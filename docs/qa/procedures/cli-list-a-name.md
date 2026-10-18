# cli-list-a-name

The Shakedex CLI (the node's hsd wallet) locks one of its names and writes a listing file; or takes a name it listed back out of its lock.

- **Parameters:** `NAME`, owned by the hsd wallet; the price or prices; `OUT`, the listing file path.
- **Blocks mined:** 13 for a listing, 13 for a cancel.
- **Requires:** `scripts/regtest.sh --with-wallet`, `SHAKEDEX_WORK` exported ([environment](../environment.md#variables)).

## Commands

| Command | What it does | Blocks |
|---|---|---|
| `NAME=<n> PRICE=5 OUT=$QA_DIR/<n>.json scripts/shakedex-cli-sell.sh fixed` | locks the name and writes a Buy Now listing file | 13 |
| `NAME=<n> START_PRICE=10 END_PRICE=5 OUT=$QA_DIR/<n>.json scripts/shakedex-cli-sell.sh auction` | the same as a one-day reverse auction, a step every 15 minutes | 13 |
| `scripts/shakedex-cli-sell.sh cancel <n>` | takes the CLI's name back out of its lock; needs the same `SHAKEDEX_WORK` the listing was made in | 13 |

`fixed` and `auction` print the listing path on the last line. The CLI writes its own `expiresAt`, which need not be a year ahead. Its first price step's lock time is the wall clock, so right after it is written the file may read "Not valid yet" in the app ([mine-and-sync](mine-and-sync.md#making-a-price-step-valid-median-time)).

## End state

`info NAME` shows the owner as the CLI's FINALIZE into its lock; the listing file is at `OUT`. After `cancel`, the name is back in the hsd wallet.
