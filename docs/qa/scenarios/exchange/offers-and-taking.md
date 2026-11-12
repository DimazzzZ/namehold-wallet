---
area: HNS ⇄ BTC exchange — making offers (sell and buy), the order book on the other profile, own offers, the Take dialog and its exact BTC, withdrawing an offer, the per-offer limit, an offer nobody renews
priority: P1–P3
blocks: HNS 0, BTC 0
requires:
  - procedures/exchange-setup.md
  - procedures/switch-profile.md
last_verified: "E10, E11, E12, E14 ✅ on 8d904e2 (2026-11-09); E15, E16, E37 ✅, E13 ◐ on f542de6 (2026-11-11)"
---

# Offers and taking

An offer is a signed announcement on the relay (spec §2, §8): a side, a price in sats per HNS (up to 3 decimals), and the smallest and largest trade in HNS. It stays listed while the maker's Namehold renews it (the heartbeat, every 60 s, in the app's sync of the active profile and in the daemon for every profile); the relay lists offers renewed within the last 180 s. Taking an offer signs nothing: it sends the taker's half of the terms, and the maker's daemon or app answers with its half; the trade then waits for the Buyer's Commit ([trades](trades.md)).

The BTC of a trade is `floor(doos × msat per HNS / 10^9)` sats, the same number on both sides (spec §9, `price.rs`, `exchangeAmount.ts`). Amounts below are those of the roles OS, OX and OB ([exchange-setup](../../procedures/exchange-setup.md#roles-used-by-the-cases)). In the order book and My offers a price reads without its unit (`10,000`), in dialogs with it (`10,000 sats/HNS`); HNS amounts drop trailing zeros (`1 HNS`, `2.5 HNS`).

HNS amounts cross the bridge as plain decimals with six places and no unit (`commands/exchange.rs::hns_text`: `"1.000000"`), which the page shows without trailing zeros (`1 HNS`); the secure windows write them with six places and the unit once (`5.000000 HNS`). A sentence that repeats an offer's own limit (`TAKE_BELOW_MIN`, `TAKE_ABOVE_MAX`) shows it as the bridge carries it (`1.000000 HNS`).

## E10 — Publish a sell offer

- **Priority:** P1
- **Covers:** spec §3.3 step 1, §8 `POST /v1/offers`, R16 (the window shows what is offered), R18
- **Regression of:** exchange QA finding 1 (HNS amounts carried with their unit read `—` on the page), exchange QA finding 2 (the unit written twice in the window's rows)
- **Preconditions:** [exchange-setup](../../procedures/exchange-setup.md) done; the Seller active (the signer may be locked: a sell offer needs no key).

Steps:

1. Exchange › `Make an offer…`; read the dialog.
2. `Sell HNS` (selected by default); `Price in sats per HNS` `10000`, `Smallest trade (HNS)` `1`, `Largest trade (HNS)` `20` → `Publish offer`. Read the secure window; `Confirm & Sign`.
3. Read the toast and My offers. Hover the `Live` badge.
4. `curl -s "$RELAY/v1/offers?network=regtest"`.
5. OX: `Make an offer…` → `Sell HNS`, `333.333`, `5`, `20` → `Publish offer` → read → `Confirm & Sign`.
6. Wait 3 minutes (switching to the Buyer meanwhile is fine: the daemon renews the Seller's offers). Reopen My offers (it does not poll); `curl` the offers again; `grep -c "POST /v1/offers" "$QA_DIR/relay/relay.log"`.

Expected:

- Step 1: title `Make an offer`; two choices `Sell HNS` ("You lock HNS and receive BTC") and `Buy HNS` ("You lock BTC and receive HNS"); the three fields; "The offer stays listed only while your Namehold keeps renewing it. Every trade someone starts from it still asks you to confirm in the secure window."; `Cancel`, and `Publish offer` disabled until all three fields read.
- Step 2: the button reads "Confirm in the secure window…"; secure window `Publish an offer`, message "Nothing is signed or locked now. Each trade someone starts on this offer asks you to confirm it here, and needs Namehold open.", rows `Offer` "Sell HNS for BTC", `Price` "10000 sats per HNS", `Each trade` "1.000000 HNS to 20.000000 HNS (10000 to 200000 sats)", `Relay` "http://127.0.0.1:8787".
- Step 3: toast "Offer published"; the page is on `My offers` (badge `1`); one row: Side `Sell HNS`, Price `10,000`, Amount `1 – 20 HNS`, BTC `≈ 0.00010000 – 0.00200000 BTC`, State `Live` (hover "Published on the relay and renewed by this Namehold"), `Withdraw`.
- Step 4: the list holds the offer with `"side":"sell"`, `"priceMsatPerHns":10000000`, `"minHns":1000000`, `"maxHns":20000000`, `"network":"regtest"`, and a `signature`.
- Step 5: rows `Price` "333.333 sats per HNS", `Each trade` "5.000000 HNS to 20.000000 HNS (1666 to 6666 sats)"; My offers now has two rows; OX: Price `333.333`, Amount `5 – 20 HNS`, BTC `≈ 0.00001666 – 0.00006666 BTC`.
- Step 6: both still `Live` and still listed by the relay; the relay log shows a `POST /v1/offers` for each offer every 60 to 90 s (the daemon's 60 s cycle plus its sync of both profiles; the app also renews the active profile's offers at its own sync, about every 2 minutes). No `POST /v1/offers` at all after the publishes means the daemon is not this build's ([exchange-setup](../../procedures/exchange-setup.md), step 5): an environment error, fix it and rerun the step.
- Passes if the offer is published exactly as confirmed and kept alive without the user.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E10-2-secure-window`, `E10-3-my-offers`; the relay's list.
- **Cleanup / end state:** OS and OX live.

## E11 — Publish a buy offer

- **Priority:** P2
- **Covers:** spec §3.3 step 2 (a Buyer-maker's pool of hashes, made from the phrase at publishing), §9 `exchange_create_offer`
- **Regression of:** exchange QA finding 10 (a buy offer with the signer locked opened the secure window, then failed)
- **Preconditions:** the Buyer active, the signer **unlocked**.

Steps:

1. `Make an offer…` → `Buy HNS`, `10000`, `1`, `20` → `Publish offer` → read → `Confirm & Sign`.
2. Lock the signer (readiness pill › Lock); make the same offer again (`Publish offer`); read the dialog and My offers. Unlock again.

Expected:

- Step 1: rows `Offer` "Buy HNS with BTC", `Price` "10000 sats per HNS", `Each trade` as in E10, `Relay`; toast "Offer published"; My offers row Side `Buy HNS`, `Live`.
- Step 2: no secure window opens; the dialog shows in red "Your signer is locked — click Unlock first." (the pool of hashes needs the phrase, so the signer is checked before the window); no new row in My offers, nothing new on the relay.
- Passes if a buy offer is published as confirmed.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshot `E11-1-buy-offer`.
- **Cleanup / end state:** OB live.

## E12 — The order book on the other profile; an own offer cannot be taken

- **Priority:** P1
- **Covers:** spec §3.3 step 1, `takeRefusal` (`OWN_OFFER`)
- **Regression of:** exchange QA finding 1 (the order book's Amount read `—`)
- **Preconditions:** OS and OX (E10) and OB (E11) live.

Steps:

1. The Buyer: Exchange › `Order book`. Read both cards; hover `Take` on OB.
2. [switch-profile](../../procedures/switch-profile.md) to the Seller: the same.
3. Note the time; on the Seller publish a throwaway sell offer (`10000`, `1`, `2`); switch to the Buyer and watch the order book without clicking.

Expected:

- Step 1: card `Sell HNS` ("People selling HNS for BTC. Take one to buy HNS.") with OS and OX: Price `10,000` / `333.333`, Amount `1 – 20 HNS` / `5 – 20 HNS`, BTC `≈ 0.00010000 – 0.00200000 BTC` / `≈ 0.00001666 – 0.00006666 BTC`, Maker 8 hex, `Take` enabled. Card `Buy HNS` ("People buying HNS with BTC. Take one to sell HNS.") with OB marked `Yours`, its `Take` disabled with "This is your own offer."
- Step 2: OS and OX marked `Yours` with `Take` disabled "This is your own offer."; OB takeable. Each Maker value is the same on both profiles.
- Step 3: the new offer appears on the Buyer within about 15 s plus the time of a switch (the order book rereads every 15 s). Withdraw it afterwards (E15 steps 1–2).
- A side with no offer reads "Nobody is selling HNS right now." / "Nobody is buying HNS right now." (observe when it happens).
- Passes if each profile sees the other's offers and cannot take its own.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E12-1-buyer-book`, `E12-2-seller-book`.
- **Cleanup / end state:** OS, OX, OB live; the throwaway offer withdrawn.

## E13 — The offer form refuses bad numbers

- **Priority:** P2
- **Covers:** R7 (limits), spec §3.5 (regtest: 1 000 sats to 10 BTC per trade)
- **Preconditions:** any enabled profile.

Steps:

1. `Make an offer…`; type in turn and read the field's line: price `0`; price `1.2345`; price `1e3`; smallest `0.0000001`; smallest `abc`; smallest `30` with largest `20`.
2. Price `10000`, smallest `0.01`, largest `1` → `Publish offer`. Then smallest `1`, largest `200000` → `Publish offer`.
3. `Cancel`.

Expected:

- Step 1: "Enter a price greater than 0, in sats per HNS like 250 or 250.5" (for `0` and `1e3`); "A price has at most 3 decimals of a sat"; "HNS has at most 6 decimals"; "Enter an amount greater than 0, in digits like 12.5"; under the smallest field "The smallest trade cannot be larger than the largest". `Publish offer` disabled throughout.
- Step 2: no secure window; in red under the fields "Invalid input: each trade must be worth 1000 to 1000000000 sats", both times (100 sats is too little, 2 000 000 000 sats too much). Nothing new in My offers or on the relay.
- Passes if no offer outside the policy reaches the secure window.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshot `E13-2-limits`.
- **Cleanup / end state:** no new offer.

## E14 — The Take dialog: amount checks and the exact BTC

- **Priority:** P1
- **Covers:** R7, spec §9 (exact sats, rounded down)
- **Regression of:** exchange QA finding 1 (the Take dialog refused every offer as unreadable)
- **Preconditions:** OS, OX, OB live; the Buyer active.

Steps:

1. On OS press `Take`; read the dialog.
2. Type in turn into `HNS to buy`: `0.5`, `21`, `1.0000001`, `abc`, `7.777777`, `5`. Read the line under the field after each. `Cancel`.
3. On OX: `Take` → `7.777777` → read → `Cancel`.
4. Switch to the Seller; on OB: `Take` → read the dialog → `5` → read → `Cancel`.

Expected:

- Step 1: title `Buy HNS`; `Price` `10,000 sats/HNS`; `Limits` `1 HNS to 20 HNS`; `Maker` (8 hex); field `HNS to buy`; "Nothing is signed now. Once both sides agree the terms, the trade asks you to confirm in the secure window."; `Cancel`, `Take offer` disabled. "This offer's price or limits cannot be read, so it cannot be taken." never shows for these offers.
- Step 2: "This offer takes at least 1.000000 HNS"; "This offer takes at most 20.000000 HNS"; "HNS has at most 6 decimals"; "Enter an amount greater than 0, in digits like 12.5"; for `7.777777` "You pay **0.00077777 BTC** (77,777 sats)." (77 777.77 rounded down); for `5` "You pay **0.00050000 BTC** (50,000 sats)." and `Take offer` enabled.
- Step 3: "You pay **0.00002592 BTC** (2,592 sats)." (7 777 777 doos × 333 333 msat / 10^9 = 2 592.58…).
- Step 4: title `Sell HNS`, field `HNS to sell`; `5` gives "You receive **0.00050000 BTC** (50,000 sats)."
- Passes if every bad amount is named and the BTC shown is exact and rounded down.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E14-1-dialog`, `E14-2-exact`, `E14-3-odd-price`.
- **Cleanup / end state:** nothing taken.

## E15 — Withdraw an offer

- **Priority:** P2
- **Covers:** spec §8 `POST /v1/offers/<id>/withdraw`, §9 `exchange_withdraw_offer`
- **Regression of:** exchange QA finding 9 (a withdrawn offer stayed in My offers as Paused)
- **Preconditions:** OX live; no trade of OX open.

Steps:

1. The Seller: My offers → `Withdraw` on OX.
2. Switch to the Buyer; watch the order book for 30 s. `curl -s "$RELAY/v1/offers?network=regtest"`.
3. Switch back to the Seller; reopen My offers; wait 3 minutes; reopen it again.

Expected:

- Step 1: toast "Offer withdrawn".
- Step 2: OX is gone from the Buyer's order book and from the relay's list; OS and OB stay.
- Step 3: OX is gone from My offers (the badge counts the remaining offers), before and after the 3 minutes; it is never renewed again (no new `POST /v1/offers` for it in `relay.log`). OX shown as `Paused`, or back on the relay, is ❌.
- Passes if the withdrawn offer leaves the order book for good.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E15-2-gone`, `E15-3-my-offers`; the relay's list.
- **Cleanup / end state:** OX withdrawn.

## E16 — At most three open trades per offer

- **Priority:** P3
- **Covers:** `exchange_io/job.rs` `MAX_OPEN_PER_OFFER` (3 trades negotiating or waiting for the Buyer's Commit per offer), `holds_a_slot`
- **Regression of:** exchange QA finding 21 (a trade that ended kept its slot for a day: three takes left to expire closed the offer to everyone)
- **Preconditions:** OS live with no open trade; the Buyer unlocked; the daemon running.

Steps:

1. The Buyer takes OS four times with `1` HNS each, within two minutes, confirming none.
2. Wait 3 minutes; `Refresh`; read the four cards.
3. `Cancel` each open card.
4. At once, and again after 3 minutes: the Buyer takes OS with `1` HNS; wait until the card reads "Confirm to start the trade" (`Refresh`); `Cancel`.
5. Optional (11 minutes): take OS three times with `1` and confirm none; wait until all three cards read "Trade expired" (the Buyer's funding window, 10 minutes); take OS a fourth time.

Expected:

- Step 2: three cards read "Confirm to start the trade"; one reads "Trade expired" with "The trade ended before either side locked any coins." and "The other side declined: the maker has too many trades open on this offer". (Which one is refused depends on the order the maker read them.)
- Step 3: each `Cancel` gives "Trade cancelled" ([E26](recovery.md#e26--cancel-before-the-commit-no-cancel-after-it)).
- Step 4: both takes are answered: the card reaches "Confirm to start the trade" with `Confirm and sign` enabled. "The other side declined: the maker has too many trades open on this offer" here is ❌: a cancelled trade must free its slot at once.
- Step 5: the fourth take is answered too: trades that expired hold no slot (they are followed for a day more, but do not count).
- Passes if the fourth take is declined with that reason and nothing is signed, and a slot is free again as soon as a trade ends.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshot `E16-2-declined`.
- **Cleanup / end state:** no open trade on OS.

## E37 — An offer nobody renews leaves the order book, and comes back

- **Priority:** P2
- **Covers:** spec §3.3 step 1 (the offer stays listed only while its maker renews it), `relay_server/handlers.rs` `LIVE_WINDOW_SECS` (180 s), `exchange_io/job.rs` `heartbeat`
- **Preconditions:** OS and OX live ([E10](#e10--publish-a-sell-offer)); no trade open; the Buyer exists.

Steps:

1. Settings › Node & tray: untick `Sync in background (keep wallet up to date when app is closed)`; `pgrep -fl namehold-syncd` shows no daemon of this build. Switch to the Buyer (the app renews only the active profile's offers). Note the time.
2. Wait 4 minutes without a `Sync` on the Seller. `curl -s "$RELAY/v1/offers?network=regtest"`; read the Buyer's order book.
3. Switch to the Seller; read My offers before any `Sync`.
4. Tick `Sync in background` again. Within 2 minutes `curl` the offers; read the Buyer's order book.

Expected:

- Step 2: the relay's list no longer holds OS and OX (not renewed for 180 s; record whether the active profile's own offer stayed: the app renewed it about every 2 minutes in one run and not at all in another); the Buyer's `Sell HNS` card reads "Nobody is selling HNS right now." if no other sell offer is live. The relay log has no `POST /v1/offers` for them since step 1.
- Step 3: My offers shows them `Paused` while the relay does not list them (an offer shown `Live` that no taker can see is worth a finding).
- Step 4: both are listed again with the same offer ids (a renewal re-posts the same signed offer), and takeable.
- Passes if an offer disappears when its maker stops renewing it and returns when the maker is back.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** the relay's lists with times; screenshots `E37-2-gone`, `E37-3-my-offers`.
- **Cleanup / end state:** the daemon running; OS, OX live.
