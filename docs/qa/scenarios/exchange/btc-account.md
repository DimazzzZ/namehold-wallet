---
area: HNS ⇄ BTC exchange — the BTC account (BIP 84 from the profile's phrase): receive address, balance, withdrawal
priority: P1–P3
blocks: HNS 0, BTC 2
requires:
  - procedures/exchange-setup.md
last_verified: "E8 ❌ (finding 18), E9 ✅ on 8d904e2 (2026-11-09); E36 seen as finding 19, not run as a case"
---

# The BTC account

Each enabled profile has a Bitcoin account made from its recovery phrase (`m/84'/1'/0'` off mainnet, spec §2, R15). The `BTC account` card sits on the right of the Exchange page: `Confirmed`, `Unconfirmed`, `Locked in trades` (each as BTC with 8 decimals and as sats), the `Receive address`, `Withdraw…`, and `Checked <time> UTC` under the title. It is read from the first configured Bitcoin source (`exchange_btc_balance`); on regtest that is the bitcoind of [exchange-setup](../../procedures/exchange-setup.md), which the app reads with `scantxoutset`, i.e. the UTXO set only. The card rereads on `Refresh`, after a Commit or a withdrawal, and when the page is opened again; it has no timer.

## E8 — Receive address and balance

- **Priority:** P1
- **Covers:** R15, spec §9 `exchange_btc_balance`
- **Regression of:** exchange QA finding 18 (with the Bitcoin source down the card showed a zero balance and "Checked" at the failed attempt's time)
- **Preconditions:** the Buyer, enabled, the regtest settings stored; no trade open (step 4 stops bitcoind).

Steps:

1. Read the card. `B getaddressinfo <the receive address>`.
2. `BW sendtoaddress <the receive address> 0.002` (do not mine). `Refresh`; read the card.
3. `bmine 1`. `Refresh`; read.
4. `bash scripts/btc-regtest.sh stop`; `Refresh`; read. `bash scripts/btc-regtest.sh start`; `Refresh`; read.
5. Optional (P3): with a BIP 84 wallet that supports regtest and the Buyer's throwaway phrase (mode B only, [rules](../../rules.md#secrets-and-evidence)), derive `m/84'/1'/0'/0/0`.

Expected:

- Step 1: `Receive address` starts `bcrt1q` and is 44 characters; `getaddressinfo` reads `"iswitness": true`, `"witness_version": 0` and a 40-hex `witness_program` (P2WPKH). `Locked in trades` `0.00000000 BTC` / `0 sats` with no trade committed.
- Step 2: `Unconfirmed` stays `0 sats` and `Confirmed` is unchanged: a Bitcoin Core source lists only confirmed coins (`scantxoutset`; spec §6 "Known gaps"). An unconfirmed amount here means the code reads otherwise: record it.
- Step 3: `Confirmed` higher by exactly `0.00200000 BTC` / `200,000 sats`; `Checked` shows the new time.
- Step 4: with bitcoind stopped the card shows a red line under the amounts (record the exact text; it names the source without credentials); `Confirmed` and `Unconfirmed` read `Not read` (never `0.00000000 BTC` / `0 sats`: a source error shown as an empty account is exchange QA finding 18 again), `Locked in trades` keeps its amount (it comes from the stored trades), and the card's subtitle reads "Last check failed <time>" instead of "Checked <time>". After the restart the amounts are back and the red line is gone.
- Step 5: the derived address equals the card's address while no trade has taken a new one (each Buyer Commit and Seller Commit takes the next receive address for a refund or a claim). Without such a wallet: BLOCKED ✗.
- Passes if the address is a regtest P2WPKH address of the account and the confirmed balance moves by exactly what was paid.

- **Blocks mined:** HNS 0, BTC 1
- **Evidence:** screenshots `E8-1-card`, `E8-2-unconfirmed`, `E8-3-confirmed`, `E8-4-source-down`; `getaddressinfo` output.
- **Cleanup / end state:** bitcoind running; the Buyer 0.002 BTC richer.

## E9 — Withdraw BTC

- **Priority:** P1
- **Covers:** spec §9 `exchange_btc_withdraw`, R16 (the secure window shows what leaves and where)
- **Regression of:** exchange QA finding 10 (a withdrawal with the signer locked opened the secure window, then failed)
- **Preconditions:** the Buyer with at least 0.005 BTC confirmed; the signer unlocked.

Steps:

1. `DEST=$(BW getnewaddress)`. `Withdraw…`; read the dialog.
2. In the dialog, in turn: `Withdraw` with every field empty; amount `abc`; amount `0`; amount `999999999`; then address `bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq` (mainnet) with amount `100000` → `Withdraw`; address `xyz` → `Withdraw`.
3. Address `$DEST`, amount `100000` → `Withdraw`. Read the secure window; `Cancel`.
4. `Withdraw` again → `Confirm & Sign`. Read the toast.
5. `B getrawmempool`; for the txid that starts with the toast's 12 hex: `B getrawtransaction <txid> true`, `B getmempoolentry <txid>`.
6. `bmine 1`; `Refresh`; `BW getreceivedbyaddress $DEST`.
7. Lock the signer (readiness pill › Lock); start another withdrawal of `10000` to `$DEST` → `Withdraw`. Unlock again after.

Expected:

- Step 1: title `Withdraw BTC`; `Bitcoin address`, `Amount in sats`, `Send everything`, `Cancel`, `Withdraw`.
- Step 2: "Enter a Bitcoin address"; "Enter a whole number of sats greater than 0, like 25000" (for `abc` and `0`); "That is more than your confirmed balance of <confirmed> sats" (the confirmed balance with thousands separators, e.g. `1,200,000 sats`); then, with no secure window: "Invalid input: 'bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq' is not an address of Regtest"; "Invalid input: 'xyz' is not a Bitcoin address: <the parser's reason>" (record the reason).
- Step 3: secure window `Send BTC`, message "From this wallet's BTC account.", rows `To` = `$DEST`, `Amount` "0.00100000 BTC (100000 sats)", `Network fee` "<BTC> (<n> sats)", `Change back to this wallet` "<BTC> (<n> sats)". `Cancel` → the dialog shows "Confirmation declined"; `B getrawmempool` has nothing new.
- Step 4: toast "BTC sent · <first 12 hex of the txid>…"; the dialog closes.
- Step 5: the transaction pays exactly `0.00100000` to `$DEST`; its other output is the `Change back to this wallet` amount; `getmempoolentry` `fees.base` equals the `Network fee` row; inputs = 100 000 + fee + change sats.
- Step 6: `Confirmed` lower by exactly 100 000 sats plus the fee (the change counts again once mined); `getreceivedbyaddress` `0.00100000`.
- Step 7: no secure window opens; the dialog shows in red "Your signer is locked — click Unlock first."; nothing new in `B getrawmempool`.
- Passes if every refusal is named before signing and the sent transaction is exactly what the window showed.

- **Blocks mined:** HNS 0, BTC 1
- **Evidence:** screenshots `E9-2-refusals`, `E9-3-secure-window`, `E9-4-toast`; the RPC outputs of step 5.
- **Cleanup / end state:** the Buyer 0.001 BTC plus a fee poorer; signer unlocked.

## E36 — No fee estimate: a BTC send is refused before signing

- **Priority:** P2
- **Covers:** R27 (a Commit is refused when no source gives a fee estimate), `exchange_io/btc_source.rs` `fee_rate`
- **Regression of:** exchange QA finding 19 (open: on a Bitcoin Core without fee history nothing can be sent and the user is given nothing to do)
- **Preconditions:** the Buyer with a confirmed BTC balance, unlocked; the only Bitcoin source a Bitcoin Core whose `B estimatesmartfee 2` answers no `feerate` (a fresh regtest chain, before [exchange-setup](../../procedures/exchange-setup.md) step 2 seeds the estimator: fold this case in there).

Steps:

1. `B estimatesmartfee 2`; record the reply.
2. `Withdraw…` → a regtest address (`BW getnewaddress`), `100000` → `Withdraw`.
3. `B getrawmempool`.
4. Seed the estimator (exchange-setup step 2); `Withdraw` again in the same dialog.

Expected:

- Step 2: no secure window; the dialog shows in red "Invalid input: no Bitcoin source gave a fee estimate, so nothing was signed: Node RPC error: Bitcoin source http://127.0.0.1:18443/: estimatesmartfee gave no rate" (no credentials in it). Record the text: the prefix `Node RPC error:` naming the HNS node for a Bitcoin source is part of finding 18.
- Step 3: nothing new in the mempool.
- Step 4: the secure window `Send BTC` opens with a `Network fee` row ([E9](#e9--withdraw-btc) step 3).
- Passes if nothing is signed without a fee estimate and the refusal says so. While finding 19 is open the verdict is ✅ for the refusal and the finding stays listed: a fallback (the node's minimum relay fee, or a rate the user enters) would change this case.

- **Blocks mined:** HNS 0, BTC 30 (the seeding)
- **Evidence:** screenshot `E36-2-refused`; the `estimatesmartfee` replies before and after.
- **Cleanup / end state:** the estimator seeded.
