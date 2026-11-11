---
area: HNS ⇄ BTC exchange — the BTC account (BIP 84 from the profile's phrase): receive address, balance, withdrawal
priority: P1–P3
blocks: HNS 0, BTC 2
requires:
  - procedures/exchange-setup.md
last_verified: "E9 ✅ on 8d904e2 (2026-11-09); E8 step 4 and E38 ✅ on 8d09fb7 (2026-11-10); E36 ✅ on 3acbf91 (2026-11-11, the estimator emptied by setting `regtest/fees/` aside)"
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
- Step 4: with bitcoind stopped the card shows a red line under the amounts (it starts `Bitcoin source http://127.0.0.1:18443/:`, names no credentials and has no "Node RPC error:" prefix; record the rest); `Confirmed` and `Unconfirmed` read `Not read` (never `0.00000000 BTC` / `0 sats`: a source error shown as an empty account is exchange QA finding 18 again), `Locked in trades` keeps its amount (it comes from the stored trades), and the card's subtitle reads "Last check failed <time>" instead of "Checked <time>". After the restart the amounts are back and the red line is gone.
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

## E36 — No fee estimate: the send uses the node's relay minimum and says so

- **Priority:** P2
- **Covers:** R27 (no estimate from any source: twice the highest relay minimum, shown as such; neither: nothing signed), `exchange_io/btc_source.rs` `fee_rate`, `floor_rate`, `commands/exchange.rs` `pick_btc_rate`
- **Regression of:** exchange QA finding 19 (on a Bitcoin Core without fee history every withdrawal and every Commit was refused and the user was given nothing to do)
- **Preconditions:** the Buyer with a confirmed BTC balance, unlocked; the only Bitcoin source a Bitcoin Core whose `B estimatesmartfee 2` answers no `feerate` (a fresh regtest chain, before [exchange-setup](../../procedures/exchange-setup.md) step 2 seeds the estimator: fold this case in there; on a seeded chain, stop bitcoind, move its `regtest/fees/` folder aside, start it, and put the folder back after the case).

Steps:

1. `B estimatesmartfee 2`; `B getmempoolinfo`; record `mempoolminfee` and `minrelaytxfee`.
2. `Withdraw…` → a regtest address (`BW getnewaddress`), `100000` → `Withdraw`. Read the secure window; `Confirm & Sign`.
3. `B getrawmempool`; `B getmempoolentry <txid>`.
4. Seed the estimator (exchange-setup step 2); withdraw `100000` again; read the secure window; `Cancel`.

Expected:

- Step 1: no `feerate` (`"errors": ["Insufficient data or no fee rate found"]`); both minimums `0.00000100` on a default regtest node (1 sat/vB).
- Step 2: the secure window `Send BTC` with the rows of [E9](#e9--withdraw-btc) step 3 and one more, `Bitcoin fee rate`: "2 sat/vB: your Bitcoin node has no fee estimate yet, so this is 2 times its minimum. It may confirm slowly while blocks are full". `Network fee` is the transaction's size times 2 sat/vB (a one-input, two-output send: 282 sats).
- Step 3: the transaction is in the mempool; `fees.base` equals the `Network fee` row.
- Step 4: with an estimate the window has no `Bitcoin fee rate` row.
- Passes if a node without fee history can still send, at a rate it relays, and the window says the rate is not an estimate.

- **Blocks mined:** HNS 0, BTC 30 (the seeding)
- **Evidence:** screenshots `E36-2-fallback-window`, `E36-4-estimate-window`; the RPC replies.
- **Cleanup / end state:** the estimator seeded; the Buyer 100 000 sats plus a fee poorer.

## E38 — A Bitcoin source outage does not undo ended trades

- **Priority:** P2
- **Covers:** `exchange_io/job.rs` `settled_status` (a final status is kept through a step that could not read a chain), `commands/exchange.rs` `locked_in_trades`
- **Regression of:** exchange QA finding 23 (with the source down, refunded trades read "can't check" and their BTC counted as locked in trades again)
- **Preconditions:** the Buyer with at least one trade that ended by a BTC refund within the last day ([E20](refunds-and-safety.md#e20--the-seller-never-commits-the-buyers-btc-comes-back-never-earlier) or [E24](refunds-and-safety.md#e24--a-wrong-amount-hns-lock-the-buyer-never-reveals)) and one that completed; no trade open (the case stops bitcoind for a minute).

Steps:

1. My trades: read each ended card. Read the BTC account card. `sqlite3` the run's database: `select json_extract(status_json,'$.kind') from exchange_trades where role='buyer' and committed=1 order by created_at`.
2. `bash scripts/btc-regtest.sh stop`. `Refresh`. Read the same cards, the BTC account card and the same query.
3. `bash scripts/btc-regtest.sh start`. `Refresh`. Read all three again. `B estimatesmartfee 2` (seed the estimator again if it lost its rate).

Expected:

- Step 2: every ended card keeps its headline ("Your BTC is back", "Trade complete"); the query answers the same list as in step 1 (`refunded` stays `refunded`, never `cannotCheck` or `needsAttention`); `Locked in trades` is unchanged; the BTC account reads as [E8](#e8--receive-address-and-balance) step 4 (`Not read`, "Last check failed <time>", the red line). A card may show the source's error line under its text.
- Step 3: the same list and amounts as in step 1; no second "Exchange: coins returned" banner for a trade that had already ended (the maintainer reads it; ◐ if not watched).
- Passes if an outage of the Bitcoin source changes no ended trade.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** the three query outputs; screenshots `E38-2-source-down`, `E38-3-source-back`.
- **Cleanup / end state:** bitcoind running, its fee estimator answering.

