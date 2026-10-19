---
area: Shakedex buying (seller profile buys CLI listings)
priority: P1–P3
blocks: ~60
requires:
  - environment.md (seller profile funded, CLI listings X1–X3, X4 for B13)
  - procedures/cli-list-a-name.md
  - procedures/buy-a-cli-listing.md
  - procedures/cli-fill.md
  - procedures/mine-and-sync.md
last_verified: "bc8cd4d, 2026-10-18, report kept outside the repo"
---

# Buying

The seller profile buys names the Shakedex CLI listed: import a listing file, check it on the node, buy it through the secure window, and finalize the name. Off mainnet there is no LearnHNS Market, so every listing comes from a file or pasted JSON. A listing the app itself locked is not bought (B17, [rules](../../rules.md)).

## B1 — Browse on regtest

- **Priority:** P1
- **Was:** full QA B1
- **Covers:** R5, R8 (absence), R15
- **Preconditions:** the seller profile active, sending allowed.

Steps:

1. Open Market (sidebar, or hotkey 4); stay on `Browse`.
2. Paste `https://market.learnhns.com/listing/foo` into the import box; press nothing else. Then clear the box.

Expected:

- Title `Market` with the subtitle "Names listed through Shakedex. Every listing is checked against your node before you can buy it."; tabs `Browse` and `My listings`; "There is no LearnHNS Market for this network. Import a listing file or paste one below."
- No `LearnHNS Market` list, no `Page 1 of N` pager, no `Hidden N: …` line, no loading state "Checking listings against your node…" stuck on screen, and no info alert (this profile can buy).
- With the link pasted the button reads `Import link`, is disabled, and "LearnHNS Market lists mainnet names only" shows under it.
- Passes if all of the above hold.

- **Blocks mined:** 0
- **Evidence:** screenshots `B1-1-browse`, `B1-2-link-blocked`.
- **Cleanup / end state:** the import box empty.

## B2 — Import a CLI listing file from disk

- **Priority:** P1
- **Was:** full QA B2; QA_WORKFLOWS §J (a single-bid name's lock coin worth 0 verifies)
- **Covers:** R2, R5, R7
- **Preconditions:** X1's listing file (one bid, so its lock coin is worth 0).

Steps:

1. Browse → `choose a file…` → native Open panel → X1's file.
2. If the row reads "Not valid yet", wait the time it names, `mine 11`, import again (and keep the first reading as [B14](#b14--a-listing-not-valid-yet) evidence).

Expected:

- Under `Imported listings` one row: `.<X1>`, `Buy now`, badge `From file`, green dot `Ready to buy`, price `5.000000 HNS`, an enabled `Buy`, and `Listed until <date> · stays buyable until sold or cancelled` with the date the CLI wrote.
- The zero-value lock coin verifies. `info <X1>` shows the CLI's FINALIZE into its lock as the owner.
- Passes if the row is buyable.

- **Blocks mined:** 0 (11 if not valid yet)
- **Evidence:** screenshot `B2-1-imported-row`.
- **Cleanup / end state:** the row stays until you leave Market.

## B3 — Import pasted JSON

- **Priority:** P2
- **Was:** full QA B3
- **Covers:** R2, R5
- **Preconditions:** X3's listing file.

Steps:

1. Paste the full text of X3's file into the box → `Import`.
2. Paste `{"version":2}` → `Import`.

Expected:

- Step 1: a row for X3 shaped like B2's, badge `From file`; the box clears.
- Step 2: a red error under the box starting `listing file:` (the parser's own words, e.g. "listing file: missing field `name`" (unverified wording: the text after `listing file:` comes from the JSON library)); no row is added.
- Passes if the valid paste imports and the bad one is refused without a row.

- **Blocks mined:** 0
- **Evidence:** screenshots `B3-1-pasted`, `B3-2-invalid`.
- **Cleanup / end state:** nothing stored.

## B4 — Listing file parser refusals

- **Priority:** P2
- **Was:** full QA B4
- **Covers:** R2
- **Preconditions:** X3's listing file; python3 to make edited copies (load with `json`, change one field, write back with `json.dump`).

Steps:

1. Make five copies of X3's file in `$QA_DIR` and import each with `choose a file…`: (a) the last byte of the first step's `signature` changed from `84` to `83`; (b) `"version": 1`; (c) `paymentAddr` replaced by any well-formed mainnet address (`hs1q…`); (d) `"name": "Bad Name"`; (e) the valid file with an extra field `"pad"` holding 300 000 spaces.

Expected:

- (a) "listing file: price step sighash type 0x83, expected 0x84"; (b) "listing file: version 1 is not supported (only 2)"; (c) "this listing is for mainnet, but this wallet is on regtest: open it with a mainnet wallet"; (d) `listing file: name "Bad Name" is not a valid Handshake name`; (e) "listing file: larger than 262144 bytes".
- No row is added for any of them.
- Passes if each copy is refused with its sentence.

- **Blocks mined:** 0
- **Evidence:** screenshot `B4-a-sighash`; quote the rest.
- **Cleanup / end state:** nothing stored.

## B5 — Verification verdicts

- **Priority:** P2
- **Was:** full QA B5
- **Covers:** R7
- **Preconditions:** an unspent CLI listing file (X2, or X3 before B12); X3's original file after B12.

Steps:

1. Copy the unspent file with the first step's `price` raised by 1 doo; import it.
2. After B12 (X3 bought by the CLI), import X3's original file again.

Expected:

- Step 1: red dot `Failed verification: a price step's signature does not verify`, price `—`, no `Buy`. (A tampered copy of a listing already spent reads `Already sold or cancelled.` instead; use an unspent one.)
- Step 2: grey dot `Already sold or cancelled.`, no `Buy`.
- Passes if both verdicts show and neither row is buyable.

- **Blocks mined:** 0
- **Evidence:** screenshots `B5-a-failed`, `B5-b-sold`.
- **Cleanup / end state:** nothing stored.

## B6 — Market fee lines from a file

- **Priority:** P2
- **Was:** full QA B6
- **Covers:** R10, R11
- **Preconditions:** an unspent CLI listing file (X1 before B7, or X2); `$WADDR`. The fee fields are not signed by the seller, so editing them keeps the steps valid.

Steps:

1. Make three copies with `feeAddr` and every step's `fee` set: (a) `feeAddr` = `$WADDR`, `fee` = 50000 (0.05 HNS); (b) `feeAddr` = a mainnet address, `fee` = 50000; (c) `feeAddr` = `$WADDR`, `fee` = 500.
2. Import each, press `Buy`, read the dialog, tick the fee box where it is enabled, then `Cancel`. Buy nothing.

Expected:

- (a) A `Market fee` row with a checkbox, **unticked** (a file never pre-ticks a fee) and enabled, the value `0.050000 HNS (1.00%)` for a 5 HNS price (the percentage is the fee over the price, in hundredths), and the warning "not signed by the seller and not the market's published fee — anyone could have added it". Ticking it raises `Total` by the fee plus a slightly larger `Network fee` (one more output).
- (b) The checkbox disabled, "the listing names no valid fee address, so no market fee is paid".
- (c) The checkbox disabled, "the market fee is below the dust limit, so it is not paid".
- Passes if no fee is pre-ticked and each unusable fee says why.

- **Blocks mined:** 0
- **Evidence:** screenshots `B6-a-unpublished-fee`, `B6-b-unusable`, `B6-c-dust`.
- **Cleanup / end state:** nothing bought.

## B7 — Buy a CLI listing through the secure window

- **Priority:** P1
- **Was:** full QA B7; QA_WORKFLOWS §J "Regtest: buy a listing created by the shakedex CLI end to end"
- **Covers:** R1, R10, R13, R14, R30
- **Regression of:** Q1 (Activity showed the generic `Transfer` instead of `Buy` once the chain row arrived)
- **Preconditions:** X1 imported and `Ready to buy`; note the spendable balance on the Wallet page.

Steps:

1. `Buy` → read the dialog → `Buy` → the secure window (passphrase first if asked) → read → confirm.
2. Without mining: Wallet › Owned Names; Activity filtered by X1; `scripts/regtest.sh rpc getrawmempool`.
3. `mine 1`, Sync; read Owned Names and Activity; `info <X1>`.

Expected:

- The dialog `Buy .<X1>`: `Price (to seller)` `5.000000 HNS`, `Network fee`, `Total` = price + fee, no `Market fee` row, "The name becomes yours after a finalize, 10 blocks after the purchase is mined.", no expiry alert, `Fee rate override`, `Cancel` and `Buy`.
- The secure window rows: `Action` Purchase, `Name`, `Price (to seller)` `5.000000 HNS`, `Network fee`, `Total`, `Paid to` (the CLI's payment address), `Name becomes yours` "after a finalize, 10 blocks after the purchase is mined", `Txid`; the summary does not count the name's own value as money spent.
- Toast "Purchase sent — it appears under your names as an unconfirmed purchase". Owned Names: `.<X1>` as plain text, badge `Unconfirmed purchase`, no checkbox, no `Manage`, no `Finalize`. Activity: a row labelled `Buy`. The mempool holds the purchase.
- After the block and Sync: `Awaiting finalize · 9 blocks`; Activity still labels the row `Buy`, Amount minus the price (`−5.000000 HNS`), the fee shown apart; `info <X1>` shows `transfer` at that block and the owner a TRANSFER out of the CLI's lock.
- Passes if the purchase pays exactly the price shown and is tracked as above; a missed toast is ◐.

- **Blocks mined:** 1
- **Evidence:** screenshots `B7-1-dialog`, `B7-2-secure`, `B7-3-unconfirmed`, `B7-4-activity-buy`, `B7-5-awaiting`; the mempool and `info` output.
- **Cleanup / end state:** X1 awaiting finalize (B8 continues).

## B8 — Finalize the bought name

- **Priority:** P1
- **Was:** full QA B8
- **Covers:** R14
- **Regression of:** Q1 (the buyer's FINALIZE showed the generic `Finalize`)
- **Preconditions:** B7 done.

Steps:

1. `mine 8`, Sync → read the row. `mine 1`, Sync → `Finalize` → secure window → confirm.
2. Before mining: Owned Names and Activity.
3. `mine 1`, Sync; open the name's `Manage`; `info <X1>`.

Expected:

- `Awaiting finalize · 1 block` after 8 blocks, then `Ready to finalize` with an enabled `Finalize`.
- After confirming: the toast "Finalize broadcast — <first 12 hex of the txid>…" and the info toast "The name still carries the seller's DNS records: update them once it is yours."; the row reads `Finalize · waiting for a block` with no second `Finalize`; Activity shows `Finalize purchase`.
- After the block: an ordinary owned row (a link, a checkbox, `Manage`); Manage shows the seller's TXT record `shakedex-cli-sell` and the usual owner actions enabled, `Sell…` among them (Renew may be disabled for a name just renewed; Transfer is disabled until an address is typed). Activity: `Finalize purchase`, Amount 0 with the `⤷` line. `info <X1>` shows our FINALIZE as the owner and `transfer 0`.
- Passes if the name becomes an ordinary owned name with the seller's records; a missed toast is ◐.

- **Blocks mined:** 10
- **Evidence:** screenshots `B8-1-ready`, `B8-2-waiting`, `B8-3-activity-finalize-purchase`, `B8-4-owned`, `B8-5-manage-dns`.
- **Cleanup / end state:** X1 owned by the seller profile. Don't list it and buy it back from the same phrase.

## B9 — Cancel a purchase in the secure window

- **Priority:** P1
- **Was:** full QA B9; QA_WORKFLOWS §J "Cancel the secure window"
- **Covers:** R10, R13
- **Preconditions:** a buyable row you are not about to buy (X3 before B12, or X2 before B10).

Steps:

1. `Buy` → `Buy` → in the secure window press `Cancel`.
2. At once, with no Sync, press `Buy` again; then `Cancel` the secure window again.
3. Read Owned Names, Activity and the balance.

Expected:

- The dialog shows "Confirmation declined"; no purchase appears in Owned Names; no `Buy` draft stays in Activity (the prepared purchase was discarded); the balance is unchanged.
- The second `Buy` builds and opens the secure window again with the same price, without any "already" refusal.
- Passes if declining leaves nothing behind and does not block a retry.

- **Blocks mined:** 0
- **Evidence:** screenshot `B9-1-after-cancel`.
- **Cleanup / end state:** the listing still buyable.

## B10 — Buy a reverse auction at its current step

- **Priority:** P2
- **Was:** full QA B10
- **Covers:** R3, R10, R12
- **Regression of:** Q3 (the purchase row counted the seller's name coin as spent: price plus the name's value)
- **Preconditions:** X2 imported (a name with value 0.5 HNS); B11 first if you run it, since it needs X2 unspent.

Steps:

1. Read the row; open `All N price steps`.
2. `Buy` → read the dialog and the secure window → confirm; `mine 1`, Sync.
3. Activity filtered by X2. Optionally (P3) `mine 9`, Sync, `Finalize`, `mine 1`, Sync.

Expected:

- The row: `Reverse auction`, `Ready to buy`, the price = the current step, `Next <cheaper> HNS in ~T · floor <floor> HNS`, and the disclosure listing every step as `<price> HNS — now` or `<price> HNS — in ~T`.
- The dialog's `Price (to seller)`, the secure window and the chain output to the seller all equal the step shown. Then `Unconfirmed purchase` → `Awaiting finalize · 9 blocks`.
- Activity: `Buy`, Amount minus exactly that price (not the price plus the name's 0.5 HNS value); its later FINALIZE, if done, shows `Finalize purchase`, Amount 0 with `⤷ 0.500000`.
- Passes if the purchase and its history row match the current step.

- **Blocks mined:** 1 (+10 with the finalize)
- **Evidence:** screenshots `B10-1-auction-row`, `B10-2-steps`, `B10-3-unconfirmed`, `B10-4-activity`.
- **Cleanup / end state:** X2 awaiting finalize, or owned after the optional finalize.

## B11 — The price changes between review and broadcast

- **Priority:** P2
- **Was:** full QA B11; QA_WORKFLOWS §J (reverse auction, "the price changed")
- **Covers:** R3, R10
- **Preconditions:** X2 unspent and imported (before B10), or another CLI reverse auction.

Steps:

1. `Buy` → `Buy` → the secure window opens; leave it open.
2. Wait until the next step's time shown on the row has passed (up to 15 minutes), `mine 11`.
3. Confirm in the secure window. Read the dialog, Activity and `getrawmempool`.
4. Press `Buy` again and complete the purchase (B10's checks apply).

Expected:

- Nothing is sent: the dialog shows "the price changed — review the purchase again" (possibly prefixed "Broadcast failed: Invalid input: ") and refreshes the figures to the new, cheaper step; no `Buy` draft stays in Activity; the mempool is empty and the lock coin unspent.
- The second purchase pays the new price.
- Passes if the old price is never broadcast. If the secure window times out first, record it and its message (◐).

- **Blocks mined:** 11 (+1 for the purchase)
- **Evidence:** screenshots `B11-1-price-changed`, `B11-2-no-buy-draft`.
- **Cleanup / end state:** X2 bought at the cheaper step.

## B12 — Another buyer comes first

- **Priority:** P1
- **Was:** full QA B12; QA_WORKFLOWS §J (a purchase someone else beats reads "Lost")
- **Covers:** R13
- **Preconditions:** X3 imported and `Ready to buy` (or the spare X5); note the spendable balance after a Sync.

Steps:

1. `Buy` → `Buy` → the secure window opens; do not confirm yet.
2. In a terminal: [cli-fill](../../procedures/cli-fill.md) mined at once with X3's file.
3. Confirm in the secure window; read the dialog. Close it, Sync, read Activity, Owned Names and the balance; `info <X3>`.

Expected:

- Owned Names and the Market page refresh as soon as the dialog shows the failure, without waiting for the next poll.
- The broadcast reports "The node did not take the transaction. hsd does not say why; often its coins are already spent elsewhere. You can try again; the coins stay held until the node is checked again." (or another broadcast error: quote it); the dialog may then also show `Could not price this purchase` with "this listing is already sold or cancelled" and `Buy` disabled.
- After Sync: Activity shows the purchase with the badge `Lost` and under it "someone else bought the name first, or the seller cancelled the listing — nothing was paid"; Owned Names has no row for X3; the spendable balance is back to the noted value.
- `info <X3>` shows the owner as the CLI fill's TRANSFER.
- Passes if nothing was paid and the coins are spendable again; record how many syncs or blocks the verdict took.

- **Blocks mined:** 1 (+1–6 if a later sync is needed to call it lost)
- **Evidence:** screenshots `B12-1-not-taken`, `B12-2-lost`, `B12-3-no-owned-name`.
- **Cleanup / end state:** X3 owned by the hsd wallet.

## B13 — A listing the seller cancelled

- **Priority:** P3
- **Was:** full QA B13
- **Covers:** R7
- **Preconditions:** X4 listed by the CLI with the same `SHAKEDEX_WORK`.

Steps:

1. Import X4: `Ready to buy`.
2. `scripts/shakedex-cli-sell.sh cancel <X4>` ([cli-list-a-name](../../procedures/cli-list-a-name.md)).
3. Leave Market, come back, import X4 again.

Expected:

- `Already sold or cancelled.`, price `—`, no `Buy`.
- Passes if the cancelled listing is not buyable.

- **Blocks mined:** 13
- **Evidence:** screenshot `B13-1-cancelled`.
- **Cleanup / end state:** X4 back in the hsd wallet.

## B14 — A listing not valid yet

- **Priority:** P3
- **Was:** full QA B14
- **Covers:** R3, R7
- **Preconditions:** X2's file, early in the run (its later steps still ahead).

Steps:

1. Copy X2's file with its first 2 entries of `data` removed (the rest are still signed; the first left becomes valid about 30 minutes after X2's start). Import it.
2. After the time it names has passed, `mine 11` (or let other cases' blocks do it), import it again.

Expected:

- Step 1: grey dot `Not valid yet; the first price step becomes valid in ~T.`, price `—`, no `Buy`.
- Step 2: `Ready to buy` (don't buy it if X2 is already bought).
- Passes if the future step is not buyable before its time and is after.

- **Blocks mined:** 11 (or 0 if later cases mine enough)
- **Evidence:** screenshots `B14-1-not-valid-yet`, `B14-2-ready`.
- **Cleanup / end state:** nothing bought.

## B15 — Watchlist and batch selection while a name is being bought

- **Priority:** P3
- **Was:** full QA B15; QA_WORKFLOWS §J ("a purchase on its way does not appear in Renewals")
- **Covers:** R14
- **Preconditions:** a purchase unconfirmed or awaiting finalize (during B7 or B10).

Steps:

1. Add the name to the Watchlist (or open Watchlist if it is there); read its badge.
2. In Owned Names try to select the purchase row.
3. After its finalize (B8), read the Watchlist again.

Expected:

- Before the finalize the Watchlist does not badge it `Owned`; the purchase row has no checkbox, so it cannot join `Renew Selected`.
- After the finalize the Watchlist shows it `Owned`.
- Passes if the purchase is never treated as owned before it arrives.

- **Blocks mined:** 0
- **Evidence:** screenshots `B15-1-watchlist-before-finalize`, `B15-2-watchlist-owned`.
- **Cleanup / end state:** remove the name from the Watchlist if you added it.

## B16 — A purchase draft left by a quit

- **Priority:** P3
- **Was:** full QA B16
- **Covers:** R13
- **Preconditions:** a buyable row you do not need afterwards (X2 before B10, or a spare).

Steps:

1. `Buy` → `Buy` → the secure window opens; quit the app instead of confirming (app menu Quit, or `osascript -e 'tell application "Namehold" to quit'`; [ui](../../ui.md#known-quirks-and-their-workarounds)).
2. Relaunch, unlock; read Activity and Owned Names.
3. Discard what is left (Activity `Discard`). Never press `Sign & broadcast` here.

Expected:

- Either nothing remains (the draft was discarded at quit), or Activity shows a `Buy` draft with `Sign & broadcast` and `Discard` (say which). Owned Names showing `Unconfirmed purchase` for a purchase that was never sent is a finding only if it persists after a Sync.
- After `Discard` (and its confirm "Discard this draft? Any coins it reserved will be freed. This cannot be undone.", if it appears) the draft and any purchase row go, toast "Draft discarded", Owned Names drops the `Unconfirmed purchase` row at once (no Sync needed), and the balance is unchanged.
- Passes if no unsent purchase survives a discard and no coins stay reserved.

- **Blocks mined:** 0
- **Evidence:** screenshot `B16-1-after-relaunch`.
- **Cleanup / end state:** the listing still buyable.

## B17 — Your own listing is not bought

- **Priority:** P3
- **Covers:** R33
- **Preconditions:** a listing file the seller profile exported (`$QA_DIR/SA.json` from [selling](selling.md), any state); the second-device profile ([environment](../../environment.md#profiles)).

Steps:

1. In the seller profile: Market › Browse, import `$QA_DIR/SA.json` (paste it or choose the file).
2. Hover the row's `Buy`.
3. [switch-profile](../../procedures/switch-profile.md) to the second device and import the same file again; hover `Buy`.

Expected:

- Steps 1 and 3: the row's status reads "this is your own listing: cancel it instead (My listings), buying it would only cost you network fees", whatever the listing's state on chain; `Buy` is shown, disabled, and its hint (step 2) says the same sentence. The second device of the same phrase on this machine counts as the same device.
- Nothing opens, nothing is signed: no `Buy` draft in Activity.
- Passes if neither profile can start a purchase of the listing.

- **Blocks mined:** 0
- **Evidence:** screenshots `B17-1-seller`, `B17-3-second-device`.
- **Cleanup / end state:** the seller profile active; nothing bought.
