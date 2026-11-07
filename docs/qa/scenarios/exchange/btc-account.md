---
area: HNS ⇄ BTC exchange — the BTC account (BIP 84 from the profile's phrase): receive address, balance, withdrawal
priority: P1–P3
blocks: HNS 0, BTC 2
requires:
  - procedures/exchange-setup.md
last_verified: "not run yet (written against 0fa8629)"
---

# The BTC account

Each enabled profile has a Bitcoin account made from its recovery phrase (`m/84'/1'/0'` off mainnet, spec §2, R15). The `BTC account` card sits on the right of the Exchange page: `Confirmed`, `Unconfirmed`, `Locked in trades` (each as BTC with 8 decimals and as sats), the `Receive address`, `Withdraw…`, and `Checked <time> UTC` under the title. It is read from the first configured Bitcoin source (`exchange_btc_balance`); on regtest that is the bitcoind of [exchange-setup](../../procedures/exchange-setup.md), which the app reads with `scantxoutset`, i.e. the UTXO set only. The card rereads on `Refresh`, after a Commit or a withdrawal, and when the page is opened again; it has no timer.

## E8 — Receive address and balance

- **Priority:** P1
- **Covers:** R15, spec §9 `exchange_btc_balance`
- **Preconditions:** the Buyer, enabled, the regtest settings stored; no trade open (step 4 stops bitcoind).

Steps:

1. Read the card. `B getaddressinfo <the receive address>`.
2. `BW sendtoaddress <the receive address> 0.002` (do not mine). `Refresh`; read the card.
3. `bmine 1`. `Refresh`; read.
4. `bash scripts/btc-regtest.sh stop`; `Refresh`; read. `bash scripts/btc-regtest.sh start`; `Refresh`; read.
5. Optional (P3): with a BIP 84 wallet that supports regtest and the Buyer's throwaway phrase (mode B only, [rules](../../rules.md#secrets-and-evidence)), derive `m/84'/1'/0'/0/0`.

Expected:

- Step 1: `Receive address` starts `bcrt1q` and is 44 characters; `getaddressinfo` reads `"iswitness": true`, `"witness_version": 0` and a 40-hex `witness_program` (P2WPKH). `Locked in trades` `0.00000000 BTC` / `0 sats` with no trade committed.
- Step 2: by our reading of the code `Unconfirmed` stays `0 sats` and `Confirmed` is unchanged: the bitcoind source sees only mined coins. Record what the card shows; an unconfirmed amount here would mean the code reads otherwise. (Spec §9 has an unconfirmed balance that a bitcoind source can never fill: note it as a finding if `Unconfirmed` stays 0.)
- Step 3: `Confirmed` higher by exactly `0.00200000 BTC` / `200,000 sats`; `Checked` shows the new time.
- Step 4: with bitcoind stopped the card shows a red line under the amounts (record the exact text; it names the source without credentials); record whether the amounts then read 0 (a source error shown as a zero balance is a finding). After the restart the amounts are back and the red line is gone.
- Step 5: the derived address equals the card's address while no trade has taken a new one (each Buyer Commit and Seller Commit takes the next receive address for a refund or a claim). Without such a wallet: BLOCKED ✗.
- Passes if the address is a regtest P2WPKH address of the account and the confirmed balance moves by exactly what was paid.

- **Blocks mined:** HNS 0, BTC 1
- **Evidence:** screenshots `E8-1-card`, `E8-2-unconfirmed`, `E8-3-confirmed`, `E8-4-source-down`; `getaddressinfo` output.
- **Cleanup / end state:** bitcoind running; the Buyer 0.002 BTC richer.

## E9 — Withdraw BTC

- **Priority:** P1
- **Covers:** spec §9 `exchange_btc_withdraw`, R16 (the secure window shows what leaves and where)
- **Preconditions:** the Buyer with at least 0.005 BTC confirmed; the signer unlocked.

Steps:

1. `DEST=$(BW getnewaddress)`. `Withdraw…`; read the dialog.
2. In the dialog, in turn: `Withdraw` with every field empty; amount `abc`; amount `0`; amount `999999999`; then address `bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq` (mainnet) with amount `100000` → `Withdraw`; address `xyz` → `Withdraw`.
3. Address `$DEST`, amount `100000` → `Withdraw`. Read the secure window; `Cancel`.
4. `Withdraw` again → `Confirm & Sign`. Read the toast.
5. `B getrawmempool`; for the txid that starts with the toast's 12 hex: `B getrawtransaction <txid> true`, `B getmempoolentry <txid>`.
6. `bmine 1`; `Refresh`; `BW getreceivedbyaddress $DEST`.
7. Optional (P3): lock the signer (readiness pill › Lock) and start another withdrawal of `10000` to `$DEST`; confirm the window if it opens.

Expected:

- Step 1: title `Withdraw BTC`; `Bitcoin address`, `Amount in sats`, `Send everything`, `Cancel`, `Withdraw`.
- Step 2: "Enter a Bitcoin address"; "Enter a whole number of sats greater than 0, like 25000" (for `abc` and `0`); "That is more than your confirmed balance of <confirmed> sats" (the confirmed balance with thousands separators, e.g. `1,200,000 sats`); then, with no secure window: "Invalid input: 'bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq' is not an address of Regtest"; "Invalid input: 'xyz' is not a Bitcoin address: <the parser's reason>" (record the reason).
- Step 3: secure window `Send BTC`, message "From this wallet's BTC account.", rows `To` = `$DEST`, `Amount` "0.00100000 BTC (100000 sats)", `Network fee` "<BTC> (<n> sats)", `Change back to this wallet` "<BTC> (<n> sats)". `Cancel` → the dialog shows "Confirmation declined"; `B getrawmempool` has nothing new.
- Step 4: toast "BTC sent · <first 12 hex of the txid>…"; the dialog closes.
- Step 5: the transaction pays exactly `0.00100000` to `$DEST`; its other output is the `Change back to this wallet` amount; `getmempoolentry` `fees.base` equals the `Network fee` row; inputs = 100 000 + fee + change sats.
- Step 6: `Confirmed` lower by exactly 100 000 sats plus the fee (the change counts again once mined); `getreceivedbyaddress` `0.00100000`.
- Step 7: record the order: by our reading the secure window opens and the signing then fails with "Your signer is locked — click Unlock first." and nothing is sent (a window shown for a send that cannot be signed is a UX finding). Unlock again after.
- Passes if every refusal is named before signing and the sent transaction is exactly what the window showed.

- **Blocks mined:** HNS 0, BTC 1
- **Evidence:** screenshots `E9-2-refusals`, `E9-3-secure-window`, `E9-4-toast`; the RPC outputs of step 5.
- **Cleanup / end state:** the Buyer 0.001 BTC plus a fee poorer; signer unlocked.
