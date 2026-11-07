---
area: HNS ⇄ BTC exchange — a whole trade that completes: the two Commits and their secure windows, the locks on both chains, the reveal and the claim without a click, with the app open or closed, with either party as the maker
priority: P1–P2
blocks: HNS 6, BTC 6
requires:
  - procedures/exchange-setup.md
  - procedures/switch-profile.md
  - scenarios/exchange/offers-and-taking.md (OS, OB)
last_verified: "not run yet (written against 0fa8629)"
---

# Trades that complete

A trade (spec §3.3): the taker takes an offer; the maker's Namehold answers with its half of the terms; the **Buyer** commits first (one secure window signs the BTC lock's funding, the BTC refund ladder and the HNS claim), and the BTC lock is sent; once it has its confirmation (1 on regtest) the **Seller** commits (the HNS lock's funding, the HNS refund, the BTC claim ladder) and the HNS lock is sent; once that has its confirmation the Buyer's Namehold sends its claim of the HNS, whose witness reveals the secret, and the Seller's Namehold (or the watchtower) takes the secret from the HNS chain and claims the BTC. After the two Commits nobody clicks anything.

Regtest windows (`policy.rs`), from the trade's start `t0` (when the maker answered the take): the Buyer commits by `t0 + 10 min` (`Buyer locks BTC by`), the Seller by `t0 + 20 min` (`Seller locks HNS by`), the Buyer claims by `t0 + 45 min` (`Buyer claims HNS by`), the HNS refund opens at `t0 + 60 min` (`HNS can go back to the seller from`), the BTC refund at `t0 + 120 min` (`BTC can go back to the buyer from`). A trade card shows these as `Nov 7, 2026, 14:05 UTC`; the secure windows as `2026-11-07 14:05 UTC`; `terms_times $TERMS` gives them to the second.

Who steps a trade: the daemon steps every profile every ~60 s; the app's own sync (every ~60 s) and `Refresh` step the active profile only. The trade list on the page rereads every 15 s. Allow up to 3 minutes for each "within" below; press `Refresh` on the active profile to hurry it.

The amount checks and the trade cards' HNS amounts are subject to the HNS format finding described at the top of [offers-and-taking](offers-and-taking.md): if E14 failed that way, these cases are BLOCKED ✗, or are run with the inspector's take (maintainer's call) and the `—` amounts recorded.

## E17 — The happy path: a Buyer takes a sell offer

- **Priority:** P1
- **Covers:** R1, R3, R7, R8, R9, R10, R12, R13, R16, R22, spec §3.2, §3.3
- **Preconditions:** [exchange-setup](../../procedures/exchange-setup.md) done; OS live ([E10](offers-and-taking.md#e10--publish-a-sell-offer)); `mtp` shows both chains at or before now; the daemon running (`pgrep -fl namehold-syncd`); the Buyer with at least 0.002 BTC confirmed, the Seller with at least 10 HNS spendable; record both balances (BTC card `Confirmed` of each profile, Wallet `Spendable` of each after a Sync). No other trade open (so the mempools hold only this trade's transactions).

Steps:

1. The Buyer, signer unlocked: Order book → OS `Take` → `5` → `Take offer`. Read the toast and the new card at once.
2. Within 3 minutes (press `Refresh`): read the card again, all of it.
3. `Confirm and sign`. Read the secure window, every row; record `BTC lock` as `BTC_LOCK` and `Trade` as `TERMS`. `Confirm & Sign`.
4. Read the toast and the card. Find the lock transaction: `for t in $(B getrawmempool | tr -d '[]",'); do B getrawtransaction "$t" true | grep -q "$BTC_LOCK" && echo "$t"; done` → `BTC_TX`; `B getrawtransaction $BTC_TX true`; `B getmempoolentry $BTC_TX`. Read the BTC account card. Within a minute or two: the card's `Watchtower` line; `watch_job "$TERMS-buyer"`; `terms_times $TERMS`.
5. Switch to the Seller (do not unlock yet). Read its card.
6. `bmine 1`. Within 3 minutes read the Seller's card and the `My trades` tab's badge and description.
7. Unlock; `Confirm and sign`. Read the secure window; record `HNS lock` as `HNS_LOCK`, the address in `You receive` as `SELLER_BTC`, the address in `If the buyer does not take the HNS` as `SELLER_HNS_BACK`. `Confirm & Sign`.
8. Read the toast and the card. `hns_at "$HNS_LOCK"`. Within a minute or two: `watch_job "$TERMS-seller"`. Switch to the Buyer and read its card.
9. `mine 1` (HNS). Click nothing. Within 3 minutes: `hns_at "$HNS_LOCK"` again; `btc_spender $BTC_TX 0`. Read the Buyer's card.
10. `mine 1`; `bmine 1`. Within 3 minutes read both cards (switch). Sync on each profile's Wallet page; `Refresh` on each Exchange page; read the balances.

Expected:

- Step 1: toast "Trade started · it is under My trades"; the page is on `My trades`. The card: `You buy HNS · you took it`; `5 HNS for 0.00050000 BTC`; `50,000 sats · 10,000 sats/HNS`; "Agreeing on the terms with the other side's Namehold. Nothing is signed yet."; no time rows; `BTC lock` and `HNS lock` `Not yet`; `Watchtower` "Not registered with a watchtower"; `Cancel`. Record what the headline and the deadline line say while negotiating (by our reading "Confirm to start the trade" with a disabled `Confirm and sign` whose hint is "The terms are not agreed yet.", and "Needs your confirmation before <the time of the take>": the take's own time shown as a deadline is a finding).
- Step 2: headline "Confirm to start the trade" (amber); the sentence "Confirm and sign before <Buyer locks BTC by>. That one confirmation signs every transaction of your side at once: paying your BTC into the lock, taking it back if the trade fails, and claiming the HNS. If the seller never locks the HNS, your BTC comes back to you automatically after <BTC can go back to the buyer from>."; the amber line "Needs your confirmation before <Buyer locks BTC by>"; `Confirm and sign` enabled; the five time rows `Buyer locks BTC by`, `Seller locks HNS by`, `Buyer claims HNS by`, `HNS can go back to the seller from`, `BTC can go back to the buyer from`, 10, 20, 45, 60 and 120 minutes after one start (± a minute of display rounding).
- Step 3: secure window `Lock BTC for HNS`, message "This signs every transaction of your side of the trade at once: your BTC goes into a lock the seller can open only by giving you the HNS, and comes back to you on its own if the seller does not."; rows, in order: `You pay` "0.00050000 BTC (50000 sats)"; `Bitcoin network fee` "<BTC> (<n> sats)"; `You receive` "<5 less the claim fee> HNS, to rs1q…" (a wallet address of the Buyer; doubled unit known at writing); `HNS network fee (taken from what you receive)` "<fee> HNS"; `Price` "10000 sats per HNS"; `BTC lock` `bcrt1q…` (64 characters: a P2WSH address); `When you get the HNS` "Automatically, once the seller's HNS lock has 1 confirmations, if that happens before <Buyer claims HNS by, as YYYY-MM-DD HH:MM UTC>"; `If the seller does not lock HNS` "Your BTC comes back to bcrt1q… after <BTC can go back…>, less a fee of <low> to <high> sats"; `Seller's deadline` "<Seller locks HNS by>"; `Trade` (64 hex). `You receive` + `HNS network fee` = 5 HNS exactly. Every time matches the card.
- Step 4: toast "Confirmed and signed". Card: "Locking your BTC", "Your BTC lock has 0 of 1 confirmations. If the seller never locks the HNS, your BTC comes back to you automatically after <BTC can go back…>."; `BTC lock` `<first 10 hex>…<last 8 hex>:0` matching `BTC_TX`; no `Cancel`, no `Confirm and sign`. bitcoind: `BTC_TX` is in the mempool; its `vout[0]` is exactly `0.00050000` to `BTC_LOCK`, `scriptPubKey.type` `witness_v0_scripthash`; any other output is change of the BTC account; `fees.base` equals the `Bitcoin network fee` row. BTC account: `Locked in trades` `0.00050000 BTC` / `50,000 sats`. `Watchtower` "Registered with the watchtower"; `watch_job` answers 200 JSON with a `status` and a `sent` log. `terms_times` gives `btcAmount 50000`, `hnsAmount 5000000` and the five times, which match the card to the minute.
- Step 5: the Seller's card `You sell HNS · your offer`, `5 HNS for 0.00050000 BTC`: "Waiting for the buyer's BTC", "The buyer's BTC lock has 0 of 1 confirmations. You are asked to confirm once it has all of them."; `BTC lock` filled as on the Buyer; `Cancel` offered (the Seller has not committed).
- Step 6: the Seller's card: headline "Confirm to lock your HNS" (amber), sentence "The buyer's BTC is locked and checked. Confirm and sign before <Seller locks HNS by>. That one confirmation signs every transaction of your side at once: paying your HNS into the lock, taking it back if the trade fails, and claiming the BTC. If the buyer never claims the HNS, it comes back to you automatically after <HNS can go back to the seller from>."; "Needs your confirmation before <Seller locks HNS by>"; the card is first in the list with an amber border; the tab reads `My trades` with badge `1` and "1 waiting for your confirmation". (No notification, Needs you entry or sidebar count is expected: the code has none; record any.)
- Step 7: secure window `Lock HNS for BTC`, message "This signs every transaction of your side of the trade at once: your HNS goes into a lock the buyer can open only by revealing the secret that pays you the BTC, and comes back to you on its own if the buyer does not."; rows: `You lock` "5.000000 HNS"; `HNS network fee` "<fee> HNS"; `You receive` "<BTC> (<n> sats) to bcrt1q…, less a Bitcoin fee of <low> to <high> sats (shown: the lowest)" (`<n>` = 50 000 − `<low>`); `Price` "10000 sats per HNS"; `The buyer's BTC lock` "1 confirmations, checked by proof of work"; `HNS lock` `rs1q…` (a P2WSH address); `When you get the BTC` "Automatically, as soon as the buyer takes the HNS: taking it reveals the secret that opens the BTC lock"; `If the buyer does not take the HNS` "Your HNS comes back to rs1q… after <HNS can go back…>, less a fee of <fee> HNS"; `Keep Namehold running` "Until the trade completes. If the buyer reveals, your BTC must be claimed before <BTC can go back…>: the background sync and a watchtower do it, but not while both are off"; `Trade` = `TERMS`. (Doubled HNS unit known at writing; "1 confirmations" is the code's wording.)
- Step 8: toast "Confirmed and signed". Seller's card: "Locking your HNS", "Your HNS lock has 0 of 1 confirmations. If the buyer never claims it, your HNS comes back to you automatically after <HNS can go back…>."; `HNS lock` filled; no `Cancel`. `hns_at` shows one transaction at height -1 (mempool) with an output of exactly `5000000` (doos) to `HNS_LOCK`. `watch_job "$TERMS-seller"` answers 200; the Seller's card reads "Registered with the watchtower". The Buyer's card: "Waiting for the seller's HNS", "The seller's HNS lock has 0 of 1 confirmations. Namehold claims the HNS for you once it has all of them." (its `HNS lock` line fills once the Seller's announcement arrives).
- Step 9, with no click: `hns_at` shows a second transaction spending `<lock txid>:<vout>` whose witness has 4 items: a signature ending `c1`, a 64-hex `X`, `01`, and the script `S`; `sha "$X"` equals `${S:14:64}` (`H`); its only output pays `5000000` less the claim fee to the Buyer's address of step 3's `You receive`, exactly that amount. `btc_spender` shows the Seller's claim spending `BTC_TX:0` with the witness `[signature ending 01, X (the same 64 hex), 01, script]`, `locktime 0`, one output to `SELLER_BTC` of exactly step 7's `You receive` sats (if more than 5 minutes passed, a replacement at the next rung may have taken its place: record each txid and fee). The Buyer's card: "Claiming your HNS", "Namehold is sending your claim of the HNS. It reveals the secret, which lets the seller take the BTC."; the Seller's (switch to read, or read it in step 10): "Claiming your BTC", "The secret is public. Namehold is sending your claim of the BTC and raises its fee until it is mined."
- Step 10: both cards "Trade complete" (green): the Buyer "The HNS is yours, and the seller has the BTC.", the Seller "The BTC is yours, and the buyer has the HNS." Balances against the recorded start: the Buyer's BTC `Confirmed` − (50 000 + the Bitcoin network fee) sats, `Locked in trades` 0; the Seller's BTC `Confirmed` + the claim's output; the Buyer's HNS + `You receive`; the Seller's HNS − (5 + the HNS network fee). Nothing else is in either mempool.
- Passes if the trade completes on both chains with the amounts and addresses the two secure windows showed, the secret appears first in the HNS claim, and nobody clicked after the Seller's Commit.

- **Blocks mined:** HNS 2, BTC 2
- **Evidence:** screenshots `E17-2-confirm-to-start`, `E17-3-buyer-window`, `E17-6-seller-confirm`, `E17-7-seller-window`, `E17-9-claiming`, `E17-10-complete-buyer`, `E17-10-complete-seller`; the `hns_at`, `btc_spender`, `getrawtransaction`, `getmempoolentry`, `watch_job` and `terms_times` outputs; `TERMS`, `BTC_LOCK`, `HNS_LOCK`.
- **Cleanup / end state:** the trade complete on both profiles; OS still live (a completed trade does not end the offer).

## E18 — The same trade with the app closed: only the daemon reveals and claims

- **Priority:** P1
- **Covers:** R13, spec §3.3 steps 4–7, SECURITY.md "What the daemon and the app's sync send"
- **Preconditions:** as E17. Mode B: the app is quit with its menu's Quit (or `osascript -e 'tell application "Namehold" to quit'` in mode A) and started again with the same command; `Close to tray (keep running in background)` unticked (Settings › Node & tray › System Tray), else quitting only hides the window.

Steps:

1. As E17 steps 1–4 (the Buyer takes OS for `2.5` HNS: 25 000 sats; commits). Note `TERMS`, `BTC_LOCK`, `BTC_TX`.
2. Quit the app. `pgrep -fl namehold-wallet` prints nothing (mode B: no `debug/namehold-wallet`); `pgrep -fl namehold-syncd` still prints the daemon.
3. `bmine 1`. Wait 3 minutes.
4. Start the app; make the Seller active; before pressing anything read its card. Unlock; `Confirm and sign` → `Confirm & Sign` (record `HNS_LOCK`, `SELLER_BTC`). After the toast, wait until its card reads "Registered with the watchtower" (press `Refresh`), then quit the app at once. Check as in step 2.
5. `mine 1` (HNS). Wait up to 3 minutes. `hns_at "$HNS_LOCK"`; `btc_spender $BTC_TX 0`. Check as in step 2 that the app is still not running.
6. `mine 1`; `bmine 1`. Wait 2 minutes. Start the app; read both cards (switch profiles) before any `Refresh`.

Expected:

- Step 2: the daemon outlives the app.
- Step 4: the Seller's card already reads "Confirm to lock your HNS" when the page opens (the daemon read the Buyer's announcement and checked the BTC lock while the app was closed; if it only appears after the app's first automatic sync, record that: the daemon's part is then ◐). The Commit is as E17 step 7, with 2.5 HNS and 25 000 sats.
- Step 5, with no app process: `hns_at` shows the Buyer's claim (witness `[…c1, X, 01, S]`, `sha X` = H); `btc_spender` shows the Seller's claim with the same `X`, paying `SELLER_BTC`. Only the daemon (or the watchtower: see `grep "$TERMS" "$QA_DIR/relay/relay.log"`; record which sent what) can have sent them.
- Step 6: both cards "Trade complete" at launch, balances moved as in E17 for 2.5 HNS / 25 000 sats.
- Passes if the reveal and the claim happened with the app closed.

- **Blocks mined:** HNS 2, BTC 2
- **Evidence:** `pgrep` outputs with times; screenshots `E18-4-seller-at-launch`, `E18-6-complete`; `hns_at`, `btc_spender` and relay log lines.
- **Cleanup / end state:** the app running; the daemon running.

## E19 — The roles reversed: the Seller takes a buy offer

- **Priority:** P2
- **Covers:** spec §3.3 step 2 (a Buyer-maker answers with a pooled hash, without the phrase), R14
- **Preconditions:** as E17, with OB live ([E11](offers-and-taking.md#e11--publish-a-buy-offer)) instead of OS; the Buyer's signer may stay **locked** (the daemon answers the take without the phrase).

Steps:

1. The Seller (signer locked is fine: a Seller's take needs no key): Order book → OB `Take` → `5` → `Take offer`.
2. Switch to the Buyer (do not unlock). Within 3 minutes read its card.
3. Unlock; continue as E17 steps 3–10, with the roles' cards reading `You buy HNS · your offer` and `You sell HNS · you took it`.

Expected:

- Step 1: toast "Trade started · it is under My trades"; the Seller's card `You sell HNS · you took it`, negotiating.
- Step 2: the Buyer's card `You buy HNS · your offer`, "Confirm to start the trade" with the five time rows, although its signer was locked all along (the answer used a hash pooled when OB was made).
- Step 3: as E17's expectations.
- Passes if a trade on a buy offer completes the same way.

- **Blocks mined:** HNS 2, BTC 2
- **Evidence:** screenshots `E19-2-buyer-maker`, `E19-complete`; chain outputs as E17.
- **Cleanup / end state:** the trade complete; OB live.
