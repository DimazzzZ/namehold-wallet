---
area: HNS ⇄ BTC exchange — trades that do not complete and the rules that keep the coins safe: refunds only after their time, the watchtower standing in for an absent party, the honest-wallet rule, a wrong-amount lock, no reveal after the deadline
priority: P1–P3
blocks: HNS ~40, BTC ~80 (mostly the 11-block MTP moves; path M and shared waits make it less)
requires:
  - procedures/exchange-setup.md
  - procedures/exchange-time-travel.md
  - procedures/switch-profile.md
  - scenarios/exchange/trades.md (E17's steps and checks)
last_verified: "not run yet (written against 0fa8629)"
---

# Refunds and safety rules

Spec §3.4 in practice. Each case starts a trade on OS like [E17](trades.md#e17--the-happy-path-a-buyer-takes-a-sell-offer) (2.5 HNS for 25 000 sats unless it says otherwise), then lets something not happen. Record for every trade: `TERMS` (the Commit's `Trade` row), `BTC_TX` (the BTC lock transaction, E17 step 4), `HNS_LOCK` (the Seller's `HNS lock` row), `BUYER_BTC_BACK` (the address in the Buyer's row `If the seller does not lock HNS`, with its fee range), `SELLER_HNS_BACK` and `SELLER_BTC` (the addresses in the Seller's rows `If the buyer does not take the HNS` and `You receive`), and the exact times (`terms_times $TERMS`, once the watchtower has the trade).

Refund times are reached with [exchange-time-travel](../../procedures/exchange-time-travel.md); several cases also need **real** time to pass (the engine reads this computer's clock for the Seller's deadline and the reveal deadline): the case says so. The trades are independent (every check is per lock), so the cases that wait can run side by side ([README](../../README.md#exchange-run-order)).

What a refund looks like on chain:

- **BTC refund**: spends `BTC_TX:0`; `locktime` = `btcRefundTime` exactly; witness 3 items `[signature ending 01, "", script]`; one output to `BUYER_BTC_BACK` of 25 000 sats less a fee inside the Buyer's row's range. If it is not mined within 5 minutes, the next rung of the ladder replaces it (same input, higher fee: BIP 125): record each txid.
- **HNS refund**: spends the HNS lock; `locktime` = `2147483648 + ceil(hnsRefundTime / 512)` (`python3 -c "import math; print(2147483648 + math.ceil(T/512))"`, R2); witness 3 items `[signature ending 01, "", script]`; one output to `SELLER_HNS_BACK` of 2 500 000 doos less the fee in the Seller's row.

## E20 — The Seller never commits: the Buyer's BTC comes back, never earlier

- **Priority:** P1
- **Covers:** R11, spec §3.4 row 1, §3.5 (regtest windows)
- **Preconditions:** as E17 (other trades of this file may be open).

Steps:

1. The Buyer takes OS for `2.5`, commits (record as above). `bmine 1`.
2. The Seller's card reads "Confirm to lock your HNS". Do not confirm. Read the Buyer's card.
3. Wait in real time until 2 minutes past `Seller locks HNS by`. `Refresh` on each profile; read both cards. `btc_spender $BTC_TX 0`.
4. Never early. Path M: once `btcRefundTime − 600` is within 100 minutes of now, `bash scripts/btc-regtest.sh mocktime $((BTC_REFUND - 600))`, `bmine 11`, `bash scripts/btc-regtest.sh mocktime 0`. Path R: at about 10 minutes before `BTC can go back to the buyer from`, `bmine 11`. Then `mtp` (the Bitcoin `mediantime` is below `btcRefundTime`); press `Refresh` three times over 3 minutes; `btc_spender $BTC_TX 0`; `B gettxout $BTC_TX 0 true`.
5. [exchange-time-travel](../../procedures/exchange-time-travel.md) on `btc` to `btcRefundTime`. Within 3 minutes (`Refresh`): read the Buyer's card; `btc_spender $BTC_TX 0`; `grep "$TERMS" "$QA_DIR/relay/relay.log"`.
6. `bmine 1`. `Refresh`; read the card and the BTC account.

Expected:

- Step 2: the Buyer: "Waiting for the seller to confirm", "The seller has until <Seller locks HNS by> to lock the HNS. If they never do, your BTC comes back to you automatically after <BTC can go back to the buyer from>."
- Step 3: the Buyer: "Waiting to take your BTC back", "The trade did not finish. Your BTC comes back to you automatically after <BTC can go back to the buyer from>."; the Seller: "Trade expired", "The trade ended before either side locked any coins.", no `Confirm and sign`. Nothing spends the BTC lock.
- Step 4: still "Waiting to take your BTC back"; nothing spends `BTC_TX:0`; `gettxout` returns the coin (25 000 sats as `0.00025000`). A refund in the mempool here is ❌ (R11).
- Step 5: the Buyer: "Taking your BTC back", "The trade did not finish. Namehold is sending the refund of your BTC to your BTC account."; the refund as described above. Record whether the relay log shows `watch <TERMS>-buyer: sent [BtcRefund]` too (the app and the watchtower send the same signed transaction: one txid).
- Step 6: "Your BTC is back", "The trade did not finish, and your BTC is back in your BTC account."; `Locked in trades` 0; `Confirmed` = the start − the funding fee − the refund fee.
- Passes if the BTC comes back by itself only after the chain's median time passed the refund time.

- **Blocks mined:** HNS 0, BTC 24 (1 + 11 + 11 + 1)
- **Evidence:** screenshots `E20-3-waiting-back`, `E20-4-not-yet`, `E20-6-back`; `mtp` before each jump; `btc_spender` outputs.
- **Cleanup / end state:** the Buyer refunded; the Bitcoin mocktime reset.

## E21 — The Buyer goes offline before the reveal: both coins come back, through the watchtower

- **Priority:** P1
- **Covers:** R11, R13, R19, spec §3.4 row 3 ("the Buyer never reveals"), SECURITY.md (the secret never goes to a watchtower)
- **Preconditions:** as E17; the watchtower set; `sqlite3` at hand.

Steps:

1. The Buyer takes OS for `2.5`, commits; `Refresh` until its card reads "Registered with the watchtower". `bmine 1`.
2. The Seller commits; `Refresh` until "Registered with the watchtower".
3. At once, before any HNS block is mined: Settings › Node & tray → untick `Sync in background (keep wallet up to date when app is closed)`; quit the app. `pgrep -fl namehold-wallet` and `pgrep -fl namehold-syncd` print nothing (in mode B, nothing under `debug/`).
4. `mine 1`. Wait 3 minutes. `hns_at "$HNS_LOCK"`; `watch_job "$TERMS-seller"`; `watch_job "$TERMS-buyer"`; `sqlite3 "$QA_DIR/relay/regtest.db" "select json from watch_jobs where id='$TERMS-buyer'" | python3 -c 'import json,sys; print(sorted(json.load(sys.stdin)))'`.
5. [exchange-time-travel](../../procedures/exchange-time-travel.md) on `hns` to `hnsRefundTime`. Within 3 minutes: `hns_at "$HNS_LOCK"`; `grep "$TERMS" "$QA_DIR/relay/relay.log"`. `mine 1`.
6. [exchange-time-travel](../../procedures/exchange-time-travel.md) on `btc` to `btcRefundTime`. Within 3 minutes: `btc_spender $BTC_TX 0`; the relay log again. `bmine 1`.
7. Start the app; tick `Sync in background…` again; read both cards; `hns_at "$HNS_LOCK"` once more.

Expected:

- Step 4: the lock is mined and unspent (no claim: only the Buyer's app holds the secret, and it is closed). The Seller's job's status reads `awaitingReveal`; record the Buyer job's. The Buyer's job has exactly the keys `btcClaims`, `btcLock`, `btcRefunds`, `hnsRefundHex`, `role`, `terms` (plus `hnsLock` in a build whose watch job carries the Seller's own lock coin): no secret and no HNS claim.
- Step 5: the HNS refund described above, paying `SELLER_HNS_BACK`; the relay log has `watch <TERMS>-seller: sent [HnsRefund]`. Before the time (step 4) nothing spent the lock.
- Step 6: the BTC refund described above, paying `BUYER_BTC_BACK`; the log has `watch <TERMS>-buyer: sent [BtcRefund]`. No app or daemon was running, so the watchtower sent both.
- Step 7: the Buyer: "Your BTC is back"; the Seller: "Your HNS is back", "The trade did not finish, and your HNS is back in your wallet." At no point did a transaction with a 4-item witness spend the HNS lock.
- Passes if both parties got their own coins back, each only after its refund time, with every app closed.

- **Blocks mined:** HNS 13 (1 + 11 + 1), BTC 13 (1 + 11 + 1)
- **Evidence:** `pgrep` outputs; the watch job JSON keys; `hns_at`, `btc_spender`, relay log lines; screenshots `E21-7-buyer`, `E21-7-seller`.
- **Cleanup / end state:** background sync on again; the app running; both mocktimes reset.

## E22 — The Seller is offline after the reveal: the watchtower claims the BTC

- **Priority:** P1
- **Covers:** R10, R13, spec §3.4 row 5 ("the Seller's Namehold is off after the reveal")
- **Preconditions:** as E17; the watchtower set.

Steps:

1. The Buyer takes OS for `2.5`, commits; `bmine 1`. The Seller commits; `Refresh` until "Registered with the watchtower".
2. Settings › Node & tray → untick `Sync in background…`; `pgrep -fl namehold-syncd` prints nothing. Switch to the Buyer. From here until step 5 do not make the Seller active (the app steps only the active profile, so the Seller's side runs nowhere but at the watchtower).
3. `mine 1`. Within 3 minutes (the Buyer's automatic sync, or `Refresh`): `hns_at "$HNS_LOCK"`; read the Buyer's card.
4. Within 3 more minutes: `btc_spender $BTC_TX 0`; `grep "$TERMS" "$QA_DIR/relay/relay.log"`.
5. `mine 1`; `bmine 1`. Read the Buyer's card. Switch to the Seller; tick `Sync in background…`; `Refresh`; read its card; `watch_job "$TERMS-seller"`.

Expected:

- Step 3: the Buyer's claim with `X` in its witness (as E17 step 9); the card "Claiming your HNS".
- Step 4: the Seller's claim of the BTC with the same `X`, paying `SELLER_BTC` (exactly the Seller's `You receive` sats at the first rung); the log has `watch <TERMS>-seller: sent [BtcClaim]`. Nothing else can have sent it.
- Step 5: both "Trade complete"; the watch job's status `completed`.
- Passes if the Seller is paid while its own Namehold never ran after the reveal.

- **Blocks mined:** HNS 2, BTC 2
- **Evidence:** `hns_at`, `btc_spender`, relay log lines; screenshot `E22-5-complete`.
- **Cleanup / end state:** background sync on.

## E23 — The honest-wallet rule: after its claim, the Buyer never takes the BTC back

- **Priority:** P1
- **Covers:** spec §3.4 row 4, SECURITY.md "The exchange" › Liveness ("An honest Namehold Buyer never refunds BTC after its HNS claim, whatever the delay"), `engine.rs` (a secret public on the HNS chain ends the Buyer's refund path)
- **Preconditions:** as E17; the watchtower set.

Steps:

1. The Buyer takes OS for `2.5`, commits; `Refresh` until "Registered with the watchtower". `bmine 1`.
2. Exchange › `Settings` → empty `Watchtower URL (optional)` → `Save` → the row `Watchtower` "http://127.0.0.1:8787 → none" → `Confirm & Sign`.
3. The Seller commits; `Refresh` twice; read its `Watchtower` line; `watch_job "$TERMS-seller"`.
4. Untick `Sync in background…` (daemon stopped). Switch to the Buyer and stay on it.
5. `mine 1`; within 3 minutes the Buyer's claim is in `hns_at "$HNS_LOCK"`. `mine 1`. Read the card. `B gettxout $BTC_TX 0 true`.
6. [exchange-time-travel](../../procedures/exchange-time-travel.md) on `btc` to `btcRefundTime`. Then for 5 minutes press `Refresh` once a minute. `btc_spender $BTC_TX 0`; `B gettxout $BTC_TX 0 true`; `grep "$TERMS-buyer" "$QA_DIR/relay/relay.log"`; `watch_job "$TERMS-buyer"`.
7. Let the Seller claim late: switch to the Seller. Within 3 minutes `btc_spender $BTC_TX 0`. `bmine 1`; read the Seller's card.
8. Exchange › `Settings` → `Watchtower URL (optional)` `http://127.0.0.1:8787` → `Save` → confirm. Tick `Sync in background…`.

Expected:

- Step 3: "Not registered with a watchtower"; `watch_job` answers `{"error":"no such watch job"}` (404).
- Step 5: the Buyer's card "Trade complete" (the HNS is the Buyer's; the BTC is still in its lock, unclaimed); `gettxout` returns the BTC lock coin.
- Step 6: the refund path is open (the Bitcoin `mediantime` is past `btcRefundTime`) and yet nothing spends `BTC_TX:0`: no transaction in `btc_spender`, the coin still in `gettxout`, the card still "Trade complete", no `sent [BtcRefund]` for `<TERMS>-buyer` in the log, and the Buyer's watch job's status `completed`. A refund here is ❌ and a blocker (the Buyer would hold both coins).
- Step 7: the Seller's claim, `X` the same as the HNS claim's, paying `SELLER_BTC`; its fee is the highest of the Seller's `less a Bitcoin fee of <low> to <high> sats` (past the refund time the ladder goes straight to its top rung, `fees::rung_now`); the card "Trade complete".
- Passes if, once the Buyer claimed, neither its app nor its watchtower ever spends the BTC lock, however late the Seller is.

- **Blocks mined:** HNS 2, BTC 13 (1 + 11 + 1)
- **Evidence:** `gettxout` before and after the refund time; `btc_spender` (empty, then the claim); relay log grep; screenshot `E23-6-still-complete`.
- **Cleanup / end state:** the watchtower URL set; background sync on; the Bitcoin mocktime reset.

## E24 — A wrong-amount HNS lock: the Buyer never reveals

- **Priority:** P1
- **Covers:** R1, R8, spec §3.4 row 2
- **Preconditions:** as E17; the hsd wallet with at least 2 HNS spendable (`W getbalance`).

The lock address is shown in the Seller's own secure window (`HNS lock` row) before anything is signed, and depends on the terms only (`hns_htlc::lock_address`), so the case reads it there, declines, and pays it another amount by hand from the hsd wallet: what a Seller who locks the wrong amount looks like to the Buyer.

Steps:

1. The Buyer takes OS for `2.5`, commits; `bmine 1`.
2. The Seller: `Confirm and sign` → record `HNS_LOCK` from the window → `Cancel`. Read the toast and the card.
3. `W sendtoaddress "$HNS_LOCK" 1`; `mine 1`; `hns_at "$HNS_LOCK"`.
4. The Buyer: `Refresh`; read the card.
5. For 5 minutes, `Refresh` once a minute; `hns_at "$HNS_LOCK"`.
6. Once 2 minutes past `Seller locks HNS by`, read the Seller's card (still never confirmed).
7. [exchange-time-travel](../../procedures/exchange-time-travel.md) on `btc` to `btcRefundTime`; within 3 minutes read the Buyer's card and `btc_spender $BTC_TX 0`; `bmine 1`; read again. `hns_at "$HNS_LOCK"` a last time.

Expected:

- Step 2: toast "Confirmation declined"; the card still "Confirm to lock your HNS"; nothing new in `H getrawmempool`.
- Step 3: one transaction at the lock address with an output of `1000000` (doos) to `HNS_LOCK`.
- Step 4: headline "The seller locked the wrong amount" (red); "The seller's HNS lock does not match the agreed amount or script, so Namehold will not claim it and never reveals the secret. Your BTC comes back to you automatically after <BTC can go back to the buyer from>."; `HNS lock` `Not yet`.
- Step 5: nothing ever spends the 1 HNS coin; no transaction anywhere carries a 4-item witness for this lock.
- Step 6: "Trade expired".
- Step 7: "Taking your BTC back", the BTC refund described above, then "Your BTC is back". The 1 HNS coin is still unspent.
- Passes if the Buyer neither claims a lock of another amount nor reveals the secret, and gets its BTC back.

- **Blocks mined:** HNS 1, BTC 13 (1 + 11 + 1)
- **Evidence:** screenshots `E24-2-declined`, `E24-4-wrong-amount`, `E24-7-back`; `hns_at` outputs.
- **Cleanup / end state:** 1 HNS of the hsd wallet stays at `HNS_LOCK` for good (nobody signed a refund of it): note it in the report.

## E25 — No reveal after the reveal deadline

- **Priority:** P3
- **Covers:** R8 ("before the reveal deadline"), spec §3.5 (reveal margin 15 min on regtest)
- **Preconditions:** as E17; **no HNS block may be mined by anyone** from step 1 until step 3 (about 30 minutes): run it alone, or beside cases that mine only Bitcoin.

Steps:

1. The Buyer takes OS for `2.5`, commits; `bmine 1`. The Seller commits (before `Seller locks HNS by`). Mine no HNS block.
2. Wait in real time until 2 minutes past `Buyer claims HNS by`. Read both cards (`Refresh` on each).
3. `mine 11` (one call). Within 3 minutes, `Refresh` both; read both cards; `hns_at "$HNS_LOCK"`. Wait 5 minutes; `hns_at` again.
4. [exchange-time-travel](../../procedures/exchange-time-travel.md) on `hns` to `hnsRefundTime`; within 3 minutes read the Seller's card and `hns_at`; `mine 1`. Then on `btc` to `btcRefundTime`; within 3 minutes the Buyer's card and `btc_spender $BTC_TX 0`; `bmine 1`.

Expected:

- Step 2: the HNS lock still unconfirmed; the Buyer's card reads "Waiting to take your BTC back" (past the reveal deadline it stops waiting for the lock).
- Step 3: the lock is confirmed but never claimed (no 4-item witness, before and after the 5 minutes); the Buyer still "Waiting to take your BTC back"; the Seller "Waiting to take your HNS back", "The trade did not finish. Your HNS comes back to you automatically after <HNS can go back to the seller from>."
- Step 4: the HNS refund ("Taking your HNS back", then "Your HNS is back"), then the BTC refund ("Your BTC is back"), both as described above.
- Passes if no claim leaves once the reveal deadline has passed, though the lock got its confirmation.

- **Blocks mined:** HNS 23 (11 + 11 + 1), BTC 13
- **Evidence:** `hns_at` outputs; screenshots `E25-3-buyer`, `E25-3-seller`.
- **Cleanup / end state:** both refunded; mocktimes reset.
