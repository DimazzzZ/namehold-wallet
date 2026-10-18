# cli-fill

The node's hsd wallet buys a listing file through the Shakedex CLI: either mined at once, or left in the mempool (for `Sale pending` and races).

- **Parameters:** `FILE`, a listing file (one the seller profile exported; never one of the CLI's own listings bought back by the app's own phrase, [rules](../rules.md)).
- **Blocks mined:** 1 for `fill`, 0 for the mempool-only fill.

## Mined at once

```bash
scripts/shakedex-cli-sell.sh fill FILE      # the hsd wallet buys FILE, then mines 1 block
```

The CLI takes the cheapest price step valid at the median time.

## Left in the mempool

After any earlier run of the script has installed the CLI into `$SHAKEDEX_WORK`:

```bash
cli_fill_nomine() {   # $1 = listing file
  ( sleep 1; echo y; sleep 1; echo y; sleep 2 ) | node --require "$SHAKEDEX_WORK/no-tty-shim.js" \
    "$SHAKEDEX_WORK/shakedex/bin/shakedex" --prefix "$SHAKEDEX_WORK/prefix" -n regtest -w primary -a test \
    --no-passphrase fill-auction "$1"
  scripts/regtest.sh rpc getrawmempool     # the fill must be there
}
cli_fill_nomine FILE
```

## End state

Mined: `info <name>` shows the owner as the fill's TRANSFER out of the lock, committing to an address of the hsd wallet. Mempool only: the fill is in `getrawmempool` and `info <name>` still shows the lock coin as the owner, until the next block.
