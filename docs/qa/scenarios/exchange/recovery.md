---
area: HNS ⇄ BTC exchange — cancelling, a crash right after a Commit, the relay going away, the watchtower line on the card, restarts, Activity and notifications, the browser mock
priority: P1–P3
blocks: HNS ~6, BTC ~20
requires:
  - procedures/exchange-setup.md
  - procedures/exchange-time-travel.md
  - procedures/switch-profile.md
  - scenarios/exchange/trades.md (E17's steps and checks)
last_verified: "not run yet (written against 0fa8629)"
---

# Cancelling, crashes, outages and restarts

The order of a Commit (`commands/exchange.rs`, module docs): everything the party signs is **stored first, then** its lock's funding is sent, and the funding is sent again on every step while it is unconfirmed, so a crash between the two never leaves coins in a lock without the way back. A trade can be cancelled only before this party's own Commit; after it, the trade ends on the chains, by a claim or a refund. Quit the app with its menu's Quit; kill it (mode B) with `pkill -9 -f debug/namehold-wallet`; start it again with the mode's command ([environment](../../environment.md#modes)). Variables as in [refunds-and-safety](refunds-and-safety.md).

## E26 — Cancel before the Commit; no cancel after it

- **Priority:** P1
- **Covers:** spec §9 `exchange_cancel_trade`, `src/lib/exchangeStatus.ts` `canCancel`
- **Preconditions:** OS live; the Buyer unlocked. Part (d) runs on any trade after a Commit (fold it into [E17](trades.md#e17--the-happy-path-a-buyer-takes-a-sell-offer) between its steps 4 and 5, and after its step 8); its inspector step needs mode B.

Steps:

1. (a) The Buyer takes OS for `1`; once the card reads "Confirm to start the trade", press `Cancel`. Read the toast and the card. Switch to the Seller; within 3 minutes read its card for this trade.
2. (b) The Buyer takes OS for `1` and presses `Cancel` at once, while the card still says "Agreeing on the terms with the other side's Namehold. Nothing is signed yet."
3. (c) The Buyer takes OS for `2.5`, commits; `bmine 1`. The Seller: on "Confirm to lock your HNS" press `Cancel`. Read both cards over the next 3 minutes. Then let it run like [E20](refunds-and-safety.md#e20--the-seller-never-commits-the-buyers-btc-comes-back-never-earlier) from its step 3 (the Buyer's BTC comes back after its time).
4. (d) On a committed trade: look for `Cancel` on the Buyer's card after its Commit and on the Seller's after its own. In the inspector: `await window.__TAURI_INTERNALS__.invoke("exchange_cancel_trade", { tradeId: "<the card's data-trade-id>" })`.
5. After each part: `H getrawmempool` and `B getrawmempool`.

Expected:

- (a) Toast "Trade cancelled". The card: "Trade expired", "The trade ended before either side locked any coins.", no `Cancel` and no `Confirm and sign`, and a red line which by our reading is "The other side declined: cancelled by you" (the user's own cancel worded as the other side's decline: record it as a wording finding). The Seller's card: "Trade expired" with "The other side declined: cancelled".
- (b) The same as (a); nothing was signed.
- (c) The Seller's card: "Trade expired", "The other side declined: cancelled by you" (same wording note). The Buyer's card ignores the decline after its Commit: it keeps "Waiting for the seller to confirm" (no declined line), and later goes "Waiting to take your BTC back" → "Your BTC is back" as E20.
- (d) No `Cancel` on either card once that party has committed. The inspector call is refused with "Invalid input: you have locked coins in this trade: it ends on the chains, by a claim or a refund"; the card is unchanged.
- Step 5: no transaction from (a) or (b); in (c) only the Buyer's BTC lock (and later its refund).
- Passes if a trade can be dropped before the party's Commit with nothing signed or sent, and never after it.

- **Blocks mined:** HNS 0, BTC 1 for (c) (+ its refund, counted in E20's run)
- **Evidence:** screenshots `E26-a-cancelled`, `E26-a-seller`, `E26-c-buyer-unchanged`, `E26-d-no-cancel`; the inspector's rejection.
- **Cleanup / end state:** no open trade from (a), (b); (c) ends refunded.

## E27 — Killed right after a Commit: the trade is kept and the funding is sent again

- **Priority:** P1
- **Covers:** R21 (a Commit is stored once, and nothing is sent unless it was stored), R23 (a committed Buyer's trade is never given up because no source shows its lock), `commands/exchange.rs` (store first, then send), R13
- **Preconditions:** as E17; mode B for the kill; no other trade between its Commits (part (a) stops bitcoind, part (c) stops hsd for a few minutes; the watchtower then reports errors in its log, which is expected).

Steps:

1. (a) The Buyer takes OS for `2.5`; `Confirm and sign`; while the secure window shows its rows, run `bash scripts/btc-regtest.sh stop`; then `Confirm & Sign`. Read the toast and the card (record its `BTC lock` outpoint). Kill the app. `bash scripts/btc-regtest.sh start`. Wait 3 minutes (the daemon keeps stepping). `B getrawmempool`; find the lock as in E17 step 4. Start the app; read the card. Then finish the trade as E17 steps 5–10.
2. (b) The Buyer takes OS for `2.5`; `Confirm and sign` → `Confirm & Sign`, and within a second kill the app. Start it again; read My trades; `B getrawmempool`.
3. (c) A trade at "Confirm to lock your HNS" on the Seller (E17 steps 1–6). `Confirm and sign`; while the window shows its rows, run `scripts/regtest.sh stop`; `Confirm & Sign`. Read the toast and the card. `scripts/regtest.sh --with-wallet`. Within 3 minutes `hns_at "$HNS_LOCK"`; read the card. Finish as E17 steps 9–10.

Expected:

- (a) Toast "Confirmed and signed"; the card "Locking your BTC" with its `BTC lock` outpoint and a red line "Bitcoin: the BTC lock was not sent yet (<reason>); it is sent again on every sync" (a later step may replace it with its own errors while bitcoind is down: record them). After bitcoind is back, the mempool holds exactly one transaction paying 25 000 sats to the lock, and its txid matches the card's outpoint (`<first 10>…<last 8>:0`); no other transaction spends the BTC account's coins. After the restart the card shows the same trade without the red line. Record whether the daemon or the app sent it. The trade then completes as E17.
- (b) Either the trade is in My trades with its `BTC lock` outpoint and that very transaction reaches the mempool within 3 minutes of the restart, or (killed before the store) the card is still "Confirm to start the trade" and no BTC left the account. A lock transaction in the mempool that no card knows is ❌ (coins in a lock without the presigned way back). Cancel or finish the trade after.
- (c) Toast "Confirmed and signed"; the card "Locking your HNS" with its `HNS lock` outpoint and a red line "HNS: the lock was not sent yet (<reason>); it is sent again on every sync" (or "HNS: the node did not take the lock yet; it is sent again on every sync"). After hsd is back, `hns_at` shows the funding with exactly that txid, and the trade completes.
- Passes if a crash or an outage at the moment of sending loses nothing and the very transaction signed at the Commit goes out later.

- **Blocks mined:** HNS 4, BTC 4 (two trades completed; (b) adds 2 + 2 if finished)
- **Evidence:** screenshots `E27-a-not-sent`, `E27-a-after-restart`, `E27-c-not-sent`; mempool and `hns_at` outputs.
- **Cleanup / end state:** bitcoind and hsd running; the trades complete or cancelled.

## E28 — The relay stops in the middle of a trade

- **Priority:** P2
- **Covers:** spec §3.4 ("the relay lies": it can only waste time), §3.6 (chain facts found on chain)
- **Preconditions:** a trade after both Commits with its HNS lock not yet mined (E17 at the end of its step 8); OS live.

Steps:

1. `relay_stop` ([exchange-setup](../../procedures/exchange-setup.md) step 4). On the Buyer wait 15 s; read the Order book.
2. The Seller: `Make an offer…` → `Sell HNS`, `10000`, `1`, `2` → `Publish offer` → `Confirm & Sign`. Read the dialog; then My offers (reopen it after 3 minutes).
3. `mine 1`. Within 3 minutes `hns_at "$HNS_LOCK"` and `btc_spender $BTC_TX 0`. `mine 1`; `bmine 1`; read both cards.
4. `relay_start`. Within 3 minutes reopen My offers on both profiles and the Order book; `curl -s "$RELAY/v1/offers?network=regtest"`.

Expected:

- Step 1: a red line above the order book (record the exact text; it names the relay, never a key) and empty tables ("Nobody is selling HNS right now." / "Nobody is buying HNS right now."); the rest of the page works.
- Step 2: after the secure window, the dialog shows in red the backend's "… the offer is saved but the relay refused it: <reason>" (by our reading prefixed "Node RPC error: ": the relay is not a node, a misleading prefix; record it). The offer is in My offers as `Paused`; OS too after 3 minutes (its heartbeat fails).
- Step 3: the reveal and the claim happen as in E17 step 9 with no relay (the Buyer finds the HNS lock by its address on its own node; the Seller reads the secret from the HNS chain); both cards "Trade complete"; no relay error on the cards.
- Step 4: the offers are `Live` again and listed by the relay, including the one made while it was down (record if not).
- Passes if a relay outage after the Commits costs nothing and the offers come back by themselves.

- **Blocks mined:** HNS 2, BTC 1
- **Evidence:** screenshots `E28-1-book-error`, `E28-2-saved-not-published`, `E28-3-complete`.
- **Cleanup / end state:** the relay running; withdraw the extra offer.

## E29 — The relay is down at the moment of the Buyer's Commit

- **Priority:** P2
- **Covers:** spec §3.3 step 3 (`btc_locked` announced), §3.4 ("the relay lies"); the watchtower registration retried
- **Preconditions:** as E17; a refund wait can be shared with E20.

The Buyer's announcement of its BTC lock is sent once, at the Commit, as best effort; the Seller learns the BTC lock only from it (it cannot look the lock up on Bitcoin without its txid). This case records what an outage at that moment does.

Steps:

1. The Buyer takes OS for `2.5`; once "Confirm to start the trade" shows, `relay_stop`; `Confirm and sign` → `Confirm & Sign`. `Refresh`; read the card.
2. `bmine 1`. `relay_start`. Within 3 minutes `Refresh` on the Buyer and read its `Watchtower` line; switch to the Seller and read its card; repeat until `Buyer locks BTC by` has passed.
3. Let it end like E20 from its step 3.

Expected:

- Step 1: "Locking your BTC"; the lock in bitcoind's mempool; `Watchtower` "Registering with the watchtower failed".
- Step 2: the Buyer's line becomes "Registered with the watchtower" once the relay is back (failed registrations are retried each step). The Seller's card, by our reading, never reaches "Confirm to lock your HNS": it stays "Waiting for the buyer to confirm" ("The buyer has until <Buyer locks BTC by> to confirm and lock their BTC. You have not signed anything yet.") and turns "Trade expired" after that time. Record exactly what it shows; if it never asks for the Seller's Commit, file a finding: an outage of seconds at the Buyer's Commit costs the trade and locks the Buyer's BTC for two hours.
- Step 3: the Buyer's BTC comes back after its refund time.
- Passes if the Seller is asked to commit once the relay is back (the spec's "it can only waste time"); the outcome by our reading of the code is the finding above.

- **Blocks mined:** HNS 0, BTC 13 (1 + 11 + 1)
- **Evidence:** screenshots `E29-1-failed`, `E29-2-registered`, `E29-2-seller`.
- **Cleanup / end state:** the relay running; the Buyer refunded.

## E30 — The watchtower line on the trade card

- **Priority:** P2
- **Covers:** R19, spec §8 `POST /v1/watch`, `GET /v1/watch/<id>`
- **Preconditions:** a new trade (fold into E20 before its Buyer Commit, or into E24).

Steps:

1. Before the Buyer's Commit: Exchange › `Settings` → `Watchtower URL (optional)` `http://127.0.0.1:8788` (nothing listens there) → `Save` → read the row → confirm. Read the card's `Watchtower` line.
2. Commit; `Refresh`; read the line.
3. `Settings` → `http://127.0.0.1:8787` → `Save` → confirm. `Refresh`; read the line. `watch_job "$TERMS-buyer"`; `grep "POST /v1/watch" "$QA_DIR/relay/relay.log"`.
4. `Refresh` twice more; count the `POST /v1/watch` lines again.

Expected:

- Step 1: the row `Watchtower` "http://127.0.0.1:8787 → http://127.0.0.1:8788"; the line "Not registered with a watchtower" (nothing is registered before a Commit).
- Step 2: "Registering with the watchtower failed".
- Step 3: "Registered with the watchtower"; `watch_job` answers with a `status` (record its `kind`), a `sent` log and `updatedAt`; one `POST /v1/watch` line with status 201.
- Step 4: no new `POST /v1/watch` line for this job (a registered job is not sent again).
- Passes if the card says truthfully whether a watchtower holds the trade.

- **Blocks mined:** none of its own
- **Evidence:** screenshots `E30-1-none`, `E30-2-failed`, `E30-3-registered`; the `watch_job` answer.
- **Cleanup / end state:** the watchtower URL `http://127.0.0.1:8787`.

## E31 — Everything survives a restart

- **Priority:** P2
- **Covers:** spec §6 "A lost database" (the trades live in the database), §9 `exchange_list_trades`, `exchange_my_offers`
- **Preconditions:** OS and OB live; at least one completed trade, one refunded and one open trade (for example E20's while it waits for the Seller's deadline).

Steps:

1. On each profile record: My offers (rows, states), My trades (each card's headline, amounts, times, outpoints, `Watchtower` line), the BTC account card, the Exchange settings dialog (then `Cancel`).
2. Quit the app; start it again. On each profile read the same, before and after one `Refresh`.

Expected:

- Every offer, trade card and setting is as recorded; the exchange is still enabled (no Enable banner); the login field still reads `set`. Live offers read `Live` again within about a minute (their heartbeat). The open trade continues as its own case describes.
- Passes if nothing of the exchange is lost or changed by a restart.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E31-1-before`, `E31-2-after`.
- **Cleanup / end state:** the app running.

## E32 — Activity, the Wallet page and notifications

- **Priority:** P3
- **Covers:** spec §3.4 ("the app notifies at the reveal"), [ui](../../ui.md#where-things-are) (Activity, Needs you)
- **Preconditions:** after E17 and one refund case (E20 or E21).

Steps:

1. On each profile open Activity; search and filter for the trade's HNS transactions (the Seller's lock funding and refund, the Buyer's claim; their txids from `hns_at`).
2. Wallet page: `Needs you`, the balance card; the sidebar's `Exchange` item, during a trade that waits for this profile's Commit.
3. `grep "exchange:" "$QA_DIR/tauri-dev.log"`; note any macOS notification from Namehold during the trades (the maintainer reads it; never screenshot it, [rules](../../rules.md#secrets-and-evidence)).

Expected:

- Step 1: the code has no exchange-specific Activity rows, labels or actions: record how each HNS transaction shows (an ordinary row of the wallet's address with the chain's generic label, or nothing) and its amount. No Bitcoin transaction appears in Activity. A `Speed up`, `Retry` or `Discard` on these rows would be a finding (they have no draft).
- Step 2: no exchange entry in `Needs you` and no count on the sidebar's `Exchange` (only the `My trades` tab's badge on the Exchange page): record. The Spendable HNS moves by what the trade moved, after a Sync.
- Step 3: the trade's events (needs commit, completed, refunded) are only written to the log by our reading; no notification is posted. Spec §3.4 says the app notifies at the reveal: if no notification ever comes, file it as a finding.
- Passes if the HNS side of a trade is accounted for and nothing claims to be an exchange action that is not.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E32-1-activity-buyer`, `E32-1-activity-seller`, `E32-2-wallet`; the log lines.
- **Cleanup / end state:** none.

## E33 — The Exchange page in the browser mock

- **Priority:** P3
- **Covers:** UI structure only (mode C, `src/lib/webqa-exchange.ts`); every status has a headline
- **Preconditions:** mode C (`pnpm dev`), [environment](../../environment.md#modes). The mock has no secure window (a command that would open one is taken as confirmed) and no chain; a Commit moves its trade one step, `Refresh` moves every trade one step.

Steps:

1. Open `/exchange`. `Enable exchange`. Open `Settings`; `Cancel`.
2. `Make an offer…` (`Sell HNS`, `250`, `1`, `100`) → `Publish offer`; read My offers.
3. Take an offer of the mock's order book for `10`; then press `Confirm and sign` or `Refresh` until the card reads "Trade complete", reading the headline at each step.

Expected:

- Step 1: no amber banner; toast "Exchange enabled".
- Step 2: toast "Offer published"; the row `Live`.
- Step 3: every step shows one of the headlines of `src/lib/exchangeStatus.ts` (never "Unknown status" or "This trade needs attention"), the time rows render, and the last is "Trade complete".
- Passes if the page renders every state the mock walks through.

- **Blocks mined:** none (no chain)
- **Evidence:** screenshots `E33-3-steps`.
- **Cleanup / end state:** stop `pnpm dev` if you started it.

## E34 — The deadline passes while the Commit window is open

- **Priority:** P2
- **Covers:** R24 (a Commit re-checks its deadline after the secure window), R9
- **Preconditions:** as E17; OS live.

Steps:

1. The Buyer takes OS for `1`; on "Confirm to start the trade" press `Confirm and sign` and leave the secure window open. Wait in real time until 1 minute past `Buyer locks BTC by` (at most 10 minutes after the start). `Confirm & Sign`. Read the toast and the card; `B getrawmempool`.
2. Optional: the same on the Seller's side: a trade at "Confirm to lock your HNS", its window left open until 1 minute past `Seller locks HNS by`, then `Confirm & Sign`; `H getrawmempool`.

Expected:

- Step 1: a red toast "Invalid input: the deadline passed while the window was open: nothing was sent"; nothing new in bitcoind's mempool; the card turns "Trade expired" ("The trade ended before either side locked any coins.") within a step; `Locked in trades` unchanged.
- Step 2: the same sentence; nothing new in hsd's mempool; the Buyer of that trade later gets its BTC back as in E20.
- Passes if a Commit confirmed after its deadline signs into nothing that is sent.

- **Blocks mined:** HNS 0, BTC 0 (step 2: 1, plus its refund)
- **Evidence:** screenshot `E34-1-too-late`; the mempool outputs.
- **Cleanup / end state:** the trade expired.
