---
area: Shakedex selling (seller profile)
priority: P1–P3
blocks: ~70
requires:
  - environment.md (seller profile funded, names SA–SD)
  - procedures/lock-and-list.md
  - procedures/cancel-and-bring-home.md
  - procedures/cli-fill.md
  - procedures/mine-and-sync.md
last_verified: "bc8cd4d, 2026-10-18, report kept outside the repo"
---

# Selling

A seller locks an owned name, sets its price at Finalize & sign, shares the listing file, and either sells it to the Shakedex CLI or cancels it and brings the name home. States, buttons and price lines are in [ui.md](../../ui.md#listing-states); the rules of every run are in [rules.md](../../rules.md).

## S1 — A Listed name's name actions point to My listings

- **Priority:** P1
- **Was:** full QA S1; T7 recheck F8; T7 S1.7
- **Covers:** R19, R27
- **Regression of:** F8 (Manage on a Listed name showed the generic "already registered" details), N2 (a Listed name's Manage said its owner coin had not synced locally)
- **Preconditions:** a `Listed for sale` name in the seller profile (SB before S3, or SE during R1); optionally names in other listing states at hand.

Steps:

1. Owned Names → `Manage` on the Listed name; read the Ownership section and hover each owner action.
2. Auctions → look the name up → open its actions; read the same.
3. Press **Sync**, wait, open `Manage` again.
4. If at hand, open the name actions of a name that is `Locking · N blocks`, `Ready to Finalize & sign`, `Cancelling`, `Cancelled · finalize to bring it home`, and of one back home after `Cancelled` or `Lock aborted`.

Expected:

- The info box reads "Its owner coin is in the Shakedex lock; manage the listing in Market › My listings."; "This name is yours, but its owner coin hasn't synced locally yet." is **not** shown, before or after the Sync.
- Update, Transfer, Renew, Revoke and `Sell…` are shown disabled, the hover reason `this name is locked for sale through Shakedex`; Sign message may be absent.
- Locking and Ready: Transfer, Renew and Revoke disabled with the same reason, `Cancel transfer` enabled (it is the R19 abort, [X2](persistence-and-recovery.md)); Update may be hidden by the pending transfer. Cancelling shows the known parked sentence ([ui](../../ui.md#known-issues-not-to-report-again)). Back home: the ordinary owner actions, enabled.
- Passes if the Listed name shows the lock hint and every owner action refused with the Shakedex reason on both paths; hover text that cannot be read is ◐.

- **Blocks mined:** 0
- **Evidence:** screenshots `S1-1-listed-manage-wallet`, `S1-2-listed-manage-auctions`; the exact info-box text.
- **Cleanup / end state:** nothing changed.

## S2 — Buy Now: lock, Finalize & sign, Listed, cancel and bring home

- **Priority:** P1
- **Was:** full QA S2; T7 S1 (S1.1, S1.2, S1.4–S1.6, S1.8), S2.1–S2.4, S5; T7 recheck F2, F3
- **Covers:** R19, R20, R23 (export), R27, R28, R29, R31 (no warning on a fresh name)
- **Regression of:** F2 (My listings showed `Locking` without the block count), F3 (Finalize home was enabled before the cancel's lockup ended)
- **Preconditions:** SA an ordinary owned name of the seller profile, signer unlocked or the passphrase at hand.

Steps:

1. Owned Names → `Manage` on SA → Ownership: `Sell…` is next to Transfer and enabled. Press it and read the Lock dialog `Sell .SA` without changing anything.
2. Pick `Buy Now — one price` → `Lock name` → `Sign and send the lock` → secure window → confirm. Before mining, open Activity filtered by SA.
3. `mine 1`, Sync. Read the SA row on Owned Names, press its `My listings` button, read the SA row there.
4. `mine 9`, Sync → `Finalize & sign` → `Price (HNS)` `5` → `Finalize & sign` → secure window → confirm. Before mining read My listings and Activity.
5. `mine 1`, Sync. Read the Listed row; `Export listing file` → `$QA_DIR/SA.json`.
6. `Cancel listing` → read the dialog → `Sign and send the cancel` → read the `Confirm cancel` secure window → confirm. Before mining read the row and Activity; `Export listing file` → `$QA_DIR/SA-cancelling.json`.
7. `mine 1`, Sync → read the row; export once more.
8. `mine 9`, Sync → `Finalize home` → read the secure window → confirm. Before mining read the row and Activity.
9. `mine 1`, Sync → read My listings, Owned Names and SA's name actions; `info SA`.

Expected:

- Step 1: the Lock dialog offers the legend `How to sell` with `Buy Now — one price` and `Reverse auction — the price falls step by step to a floor`; there is **no price field**; it says "You set the price, or the schedule, at Finalize & sign, once the transfer lockup is over."; it shows the costs sentence "Once the name is finalized into the lock, its DNS records cannot be changed and it cannot be renewed; the listing lasts at most one renewal window from that finalize, which renews the name." and `Shared as a file only: LearnHNS Market lists mainnet names only`; there is no publish checkbox, no publish privacy note and no near-expiry warning.
- Step 2: after `Lock name` the dialog shows a `Network fee` row and the buttons `Back` and `Sign and send the lock`; the secure window lists the TRANSFER (`Name action` shakedex_lock); toast "Lock sent. Once the transfer lockup is over, set the price with Finalize & sign in Market › My listings."; Activity shows the draft `Lock for sale`.
- Step 3: Owned Names and My listings both read `Locking · 9 blocks` (the same N); the `My listings` button opens Market with the My listings tab; the row reads `Buy now`, `Finalize & sign` disabled with "Finalize & sign opens after the transfer lockup", no price line.
- Step 4: the dialog `Finalize & sign .SA` has the single field `Price (HNS)`, no schedule fields, and the sentence "Each signature lets anyone buy the name at its price until the listing is cancelled and the cancel is mined. A price can be lowered later, never raised."; the secure window `Confirm Finalize & sign` lists the finalize fee and `Price step 1` `5.000000 HNS, valid at once`; toast "Finalize sent. The listing becomes Listed for sale once the finalize is mined."; the row reads `Finalizing into the lock` with no buttons; Activity shows `Finalize into lock`.
- Step 5: `Listed for sale`, `Buy now`, `Price 5.000000 HNS`, the status line `Shared as a file only: LearnHNS Market lists mainnet names only`, the buttons `Lower price`, `Cancel listing`, `Export listing file` and **no** `Publish on LearnHNS Market`; no `All N price steps` disclosure (one step); Owned Names reads `Listed for sale`. Export opens the native Save panel "Save the listing file" without asking to unlock; toast "Listing file saved"; the file is JSON with `version` 2, SA's name and one entry in `data`.
- Step 6: the dialog `Cancel the listing of .SA` reads, in order, "Anyone holding the listing file can still buy the name at its current price until this cancel is mined.", "If a purchase of the name is already in your node's mempool, the node does not take this cancel and it is never mined, although the node still answers it with a transaction id.", "Once the cancel is mined and the transfer lockup is over, finalize it to bring the name home." and "The secure window shows the current price and the fee before anything is signed.", with `Keep the listing` and `Sign and send the cancel`, and **no** "Told the listing is cancelled…" sentence. `Confirm cancel` shows `Action` "Cancel the listing", `Network fee (cancel)`, `Current price` `5.000000 HNS`, `Name comes home to` an `rs1…` address, `Lock address`, `Until it is mined`, `A purchase already sent`, `Then`. Toast "Cancel sent. Finalize the name home once it is mined and the lockup is over."; row `Cancelling`, `Lower price` disabled with "a cancel of this listing is not mined yet: its price cannot be lowered", `Export listing file` enabled; Activity shows `Cancel listing`.
- Step 7: `Cancelled · finalize to bring it home`, `Finalize home` **disabled** with "the cancel can be finalized in 9 blocks" (before a sync has counted it: "the cancel is mined, but its transfer lockup is not counted yet: sync, then finalize it"), export still works.
- Step 8: the secure window's action is `shakedex_cancel_finalize`, with "This brings the name out of its lock to the address of this wallet the cancel committed to; the listing ends."; toast "Sent. The name comes home once the finalize is mined."; row `Coming home` with `Export listing file` only; Activity shows `Finalize cancel`.
- Step 9: My listings `Cancelled`, no buttons; SA is an ordinary Owned Names row with `Manage` and no listing badge; its owner actions are enabled (Renew may be disabled for a name just renewed by the FINALIZE); `info SA` shows `transfer 0` and the owner at the step 6 `Name comes home to` address.
- Passes if every state, label, button and sentence above is seen; a missed transient toast is ◐.

- **Blocks mined:** 22
- **Evidence:** screenshots `S2-1-lock-dialog`, `S2-2-locking`, `S2-4-activity-finalize-into-lock`, `S2-5-listed`, `S2-6-cancel-buy-now-text`, `S2-6-secure-cancel`, `S2-7-awaiting-home`, `S2-8-activity-finalize-cancel`, `S2-9-home`; the three exported files; `info SA` before and after.
- **Cleanup / end state:** SA is an ordinary owned name again; its listing row reads `Cancelled`.

## S3 — Sale pending, a cancel and a lower price refused while a purchase is in the mempool, then Sold

- **Priority:** P1
- **Was:** full QA S3; T7 S2 (S2.5–S2.7)
- **Covers:** R22, R26, R28, R30
- **Preconditions:** SB an ordinary owned name; the CLI installed in `$SHAKEDEX_WORK` (any earlier [cli-list-a-name](../../procedures/cli-list-a-name.md) run).

Steps:

1. [lock-and-list](../../procedures/lock-and-list.md) SB, Buy Now 5, `EXPORT=$QA_DIR/SB.json`.
2. [cli-fill](../../procedures/cli-fill.md) left in the mempool with `$QA_DIR/SB.json`. **Do not Sync.**
3. On the still-Listed row: `Cancel listing` → `Sign and send the cancel`. Then `Lower price` → `4` → `Sign the lower price`.
4. Sync. Read My listings and Owned Names.
5. `mine 1`, Sync. Read My listings, Owned Names and Activity; `info SB`.

Expected:

- Step 3: the cancel is refused before any secure window opens, in the backend's words: "a purchase of this listing is in the node's mempool: the node would not take a cancel now" (or "the lock coin is no longer unspent (bought, cancelled, or being spent in the node's mempool): there is nothing to cancel"; quote which). Lower price is refused likewise: "a purchase of this listing is in the node's mempool: its price cannot be lowered" (or "…: the price cannot be lowered"). Nothing is signed: no `Cancel listing` draft in Activity, the row stays Listed.
- Step 4: My listings `Sale pending` with only `Export listing file` and no price line; Owned Names `Sale pending`.
- Step 5: `Sold`, no buttons, no price line, in My listings and on Owned Names (Owned Names may need one more refresh); Activity shows the 5.000000 HNS arriving with the fill's txid as a green `+5.000000`. `getrawmempool` held the fill before the block, with `info SB` still on the lock coin; after it the owner is the fill's TRANSFER.
- Passes if both refusals come before any secure window and the row walks Listed → Sale pending → Sold.

- **Blocks mined:** 12
- **Evidence:** screenshots `S3-1-cancel-refused`, `S3-1b-lower-refused`, `S3-2-sale-pending`, `S3-3-sold`; `getrawmempool` and `info SB` before and after the block.
- **Cleanup / end state:** SB is sold to the hsd wallet.

## S4 — Lower price: a price not below is refused, a lowered Buy Now is bought by the CLI at the lower price

- **Priority:** P2
- **Was:** full QA S4; T7 S4; T7 S2.6
- **Covers:** R26, R30
- **Preconditions:** SC an ordinary owned name.

Steps:

1. [lock-and-list](../../procedures/lock-and-list.md) SC, Buy Now 5, no export yet.
2. `Lower price` → type `6`.
3. Clear it, type `3` → `Sign the lower price` → read the secure window → confirm.
4. `Export listing file` → `$QA_DIR/SC.json`; count its steps: `python3 -c "import json,sys;print(len(json.load(open(sys.argv[1]))['data']))" $QA_DIR/SC.json`.
5. [cli-fill](../../procedures/cli-fill.md) mined at once with `$QA_DIR/SC.json`; Sync.

Expected:

- Step 2: the dialog `Lower the price of .SC` shows `Current price: 5.000000 HNS`, the field `New price (HNS)` and the sentence "The earlier, dearer signatures stay valid until the lock coin is spent, by a purchase or by a cancel once it is mined, so anyone holding the listing file can still use them. A price can be lowered later, never raised."; `6` is refused in red under the field with "6.000000 HNS is not below the current price 5.000000 HNS: a price can only be lowered; raising it needs a cancel" and `Sign the lower price` stays disabled; no secure window.
- Step 3: the secure window `Confirm Lower price` shows `Action` "Lower the price", `Current price` `5.000000 HNS`, `New price` `3.000000 HNS, valid at once`; toast "Price lowered"; the row reads `Buy now`, `Price 3.000000 HNS` and the disclosure `All 2 price steps` (that disclosure is correct for a lowered Buy Now).
- Step 4: the file has 2 entries in `data`.
- Step 5: `Sold`; the payment that arrives is **3.000000 HNS**, not 5 (Activity, or the hsd wallet's history).
- Passes if the dearer price is refused before signing and the CLI pays the lowered price.

- **Blocks mined:** 12
- **Evidence:** screenshots `S4-1-raise-refused`, `S4-2-secure-lower`, `S4-3-lowered`, `S4-4-sold-at-3`; the step count of `SC.json`.
- **Cleanup / end state:** SC is sold to the hsd wallet.

## S5 — A reverse auction: the schedule editor, the signed schedule, a step down, and the CLI buys at the current step

- **Priority:** P2
- **Was:** full QA S5; T7 S3 (S3.1, S3.2, S3.7–S3.9); T7 S5.1 (the reverse-auction cancel text)
- **Covers:** R3, R12 (seller side), R20, R24, R28, R30
- **Preconditions:** SD an ordinary owned name.

Steps:

1. `Manage` on SD → `Sell…` → `Reverse auction — the price falls step by step to a floor` → `Lock name` → `Sign and send the lock` → confirm; `mine 1`, Sync; `mine 9`, Sync.
2. `Finalize & sign` → read the editor; enter start `5`, floor `3`, `3` prices, `15` minutes; read the dialog.
3. `Finalize & sign` → read the secure window → confirm; `mine 1`, Sync; read the row and open `All 3 price steps`.
4. `Cancel listing` → read the first sentence → `Keep the listing`.
5. Wait until the 4 HNS step's time has passed, then `mine 11`, Sync; read the row.
6. `Export listing file` → `$QA_DIR/SD.json`; [cli-fill](../../procedures/cli-fill.md) mined at once with it; Sync.

Expected:

- Step 1: `Locking · 9 blocks` with the mode `Reverse auction` on My listings.
- Step 2: the editor has `Start price (HNS)`, `Floor price (HNS)`, `Number of prices (2 to 1000)` and `Minutes between prices (at least 15)`; with the valid schedule it shows a one-line summary ("3 prices from 5 to 3 HNS, one every 15 min; the last becomes valid ~30 min after the first.") and no warning, and does **not** list the individual prices.
- Step 3: the secure window lists every price with when it becomes valid: `Price step 1` `5.000000 HNS, valid at once`, then `Price step 2` and `Price step 3` at `4.000000 HNS` and `3.000000 HNS`, each `valid from <date time> UTC`; the Listed row reads `Now 5.000000 HNS · next 4.000000 HNS in ~T · floor 3.000000 HNS`, and the disclosure lists 3 lines `<price> HNS — from <date time>`. Steps may sit 15 to 17 minutes apart (every step after the first is rounded up to 512 s) and the first wait can read shorter than 15 minutes; neither is a finding.
- Step 4: the first sentence is "Anyone holding the listing file can still buy the name at any price step that is valid by then, a cheaper price step too once it is valid, until this cancel is mined."
- Step 5: `Now 4.000000 HNS · next 3.000000 HNS in ~T · floor 3.000000 HNS`.
- Step 6: `Sold`, paid exactly **4.000000 HNS** (the CLI takes the cheapest step valid at the median time).
- Passes if the schedule is signed as entered and the sale pays the step that was current.

- **Blocks mined:** 23
- **Evidence:** screenshots `S5-1-editor`, `S5-2-secure-schedule`, `S5-3-now-5`, `S5-4-cancel-text`, `S5-5-now-4`, `S5-6-sold-at-4`; the `data` lock times of `SD.json`.
- **Cleanup / end state:** SD is sold to the hsd wallet.

## S6 — The Lock dialog's built draft is discarded by Back, close and Escape

- **Priority:** P2
- **Was:** full QA S6; T7 S1.3; T7 recheck F1
- **Covers:** R19
- **Regression of:** F1 (Escape closed both the Lock dialog and the name's actions)
- **Preconditions:** an ordinary owned name (SA after S2, or SE before R1).

Steps:

1. `Manage` → `Sell…` → `Lock name` (the `Network fee` row shows) → `Back`. Read My listings, Activity and the balance.
2. Again `Lock name` → close the dialog with its close button. Read the same.
3. Again `Lock name` → press Escape **once**. Read the same; then press Escape again.
4. Auctions → look the name up → its actions → `Sell…` → Escape once, then once more.

Expected:

- After each of steps 1–3: no listing row for the name (or none left `Locking`), no `Lock for sale` draft in Activity, the spendable balance unchanged, and `Sell…` works again at once.
- Steps 3 and 4: the first Escape closes only the Lock dialog; the name's actions stay open. The second Escape closes the name's actions.
- Passes if no path leaves a draft or reserved coins behind and Escape closes one dialog at a time on both paths. Owner controls that still show `Locking` for a few seconds after the close, until the view refreshes, are ◐ at most.

- **Blocks mined:** 0
- **Evidence:** screenshots `S6-1-after-back`, `S6-3-after-escape`, `S6-4-auctions-first-escape`.
- **Cleanup / end state:** the name is ordinary; nothing signed.

## S7 — Declined secure windows write nothing

- **Priority:** P2
- **Was:** full QA S7
- **Covers:** R20, R26, R28
- **Preconditions:** a `Ready to Finalize & sign` row and a `Listed for sale` row (they may be the same listing at two moments, e.g. SC inside S4).

Steps:

1. On the Ready row: `Finalize & sign` → a price → `Finalize & sign` → in the secure window press `Cancel`.
2. On the Listed row: `Cancel listing` → `Sign and send the cancel` → `Cancel` in the secure window.
3. On the Listed row: `Lower price` → a lower value → `Sign the lower price` → `Cancel` in the secure window.
4. Retry one of them and confirm, to show the action still works.

Expected:

- Each decline shows "Confirmation declined" in the dialog; the row keeps its state (`Ready to Finalize & sign`, or `Listed for sale` with the same price); no new draft appears in Activity; `getrawmempool` stays empty.
- The next try opens the secure window again and completes.
- Passes if nothing is signed or stored by a declined window.

- **Blocks mined:** 0
- **Evidence:** screenshots `S7-1-declined`, `S7-2-no-draft`.
- **Cleanup / end state:** as before the case, plus whatever the confirmed retry did.

## S9 — Sell from the name link, and its hover

- **Priority:** P3
- **Was:** full QA S9; T7 S1.1 (hover)
- **Covers:** R19
- **Preconditions:** an ordinary owned name and a `Locking` name.

Steps:

1. Owned Names → click the name itself (`.name`, "View name info") → Ownership → hover `Sell…`; press it, then close the dialog without locking.
2. The same on the Locking name.

Expected:

- The same name actions dialog as `Manage`; `Sell…` enabled with no refusal on hover; pressing it opens the Lock dialog.
- On the Locking name `Sell…` is disabled, its hover `this name is locked for sale through Shakedex` (or the pending-transfer reason).
- Passes if both paths open the same dialog; an unreadable hover is ◐.

- **Blocks mined:** 0
- **Evidence:** screenshot `S9-1-sell-hover`.
- **Cleanup / end state:** nothing changed.

## S10 — Batch actions skip a locked name

- **Priority:** P3
- **Was:** full QA S10
- **Covers:** R27
- **Preconditions:** a Listed (or Locking) name and an ordinary name in Owned Names.

Steps:

1. Tick both names' checkboxes; read the batch bar; hover `Renew Selected`.
2. Clear the selection.

Expected:

- `Renew Selected` is disabled with a reason (it is offered only when every selected name may be renewed); `Transfer Selected` and the other batch buttons stay disabled for the locked name, even with a valid address typed.
- Passes if no batch action can include the locked name; an unreadable hover is ◐.

- **Blocks mined:** 0
- **Evidence:** screenshot `S10-1-renew-selected`.
- **Cleanup / end state:** selection cleared.

## S11 — Every listing pays a fresh address; relisting reuses the lock

- **Priority:** P3
- **Was:** full QA S11
- **Covers:** R21, R28
- **Preconditions:** the listing files exported in this run (S2 SA, S3 SB, S4 SC, S5 SD, R1 `SE-1.json`, R3 `SE-2.json`, R2 SF).

Steps:

1. For each file print `paymentAddr`, `publicKey`, `lockingTxHash` and `lockingOutputIdx`.
2. Compare them with each other and with the Receive Address shown when each name was locked.

Expected:

- Every listing pays a different `paymentAddr`, none equal to the Receive Address shown at its lock.
- `SE-1.json` and `SE-2.json` carry the same `publicKey` (same lock key and lock address) but a different lock outpoint and a different `paymentAddr`.
- Passes if no two listings share a payment address.

- **Blocks mined:** 0
- **Evidence:** the printed table in the report.
- **Cleanup / end state:** nothing changed.
