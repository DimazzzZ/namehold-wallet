---
area: Names — batch actions on a wallet with hundreds of names, a name in transfer, a lost auction
priority: P1–P3
blocks: ~36 setup + ~45
requires:
  - environment.md (the seller profile; the hsd wallet funded)
  - procedures/seed-many-names.md
  - procedures/mine-and-sync.md
last_verified: "never run"
---

# Batch actions, a name in transfer, a lost auction

What these cases check ([spec](../../../specs/2026-09-15-batch-name-operations.md) B3a–B3c, B9):

- A batch over 100 names goes out as a **series** of transactions of up to 100 after **one** confirmation, with a progress line in the batch bar. A failure stops the series and leaves the names not yet sent selected.
- A transfer is allowed from the chain's own facts without waiting for the capability check, and the bar **names** every selected name that blocks it, with the reason, and drops them in one click.
- A wallet with more than 200 names gets its name actions (the capability check is asked in chunks of 100).
- The confirmation shows the real fee (it used to read `~0.000000 HNS`).
- A name in transfer says so once, with when Finalize opens.
- A lost auction is shown as lost, never as owned.

Run [seed-many-names](../../procedures/seed-many-names.md) first. `NM` are its 210 names; `$WADDR` is the hsd wallet's address ([environment](../../environment.md#regtest-node-and-hsd-wallet)). Signing may open one secure window `Confirm transaction` **per transaction** of a series: confirm each one, and write down how many opened. All expected strings below are copied from the source; amounts follow [ui](../../ui.md#amounts).

## N1 — More than 200 names get their actions

- **Priority:** P1
- **Covers:** B3c
- **Regression of:** a mainnet wallet with 547 names had no actions: every row read its bare phase (`Closed`) and every batch button stayed grey with "All selected names must be transferable (owned, not mid-transfer)".
- **Preconditions:** seed-many-names done; the seller profile active; the node running and synced.

Steps:

1. Names → `Owned`. Wait up to a minute without clicking anything; take a screenshot.
2. Scroll to any `NM` row and read its status.
3. Click the header checkbox (`Select all names`) and read the batch bar.

Expected:

- Step 2: the `NM` rows read `Owned` with a muted detail (for example `renew within …`), not a bare `Closed`.
- Step 3: the bar reads `<n> selected`; `Renew Selected` and `Transfer Selected` are not greyed out because of missing data (Transfer is disabled only by an empty recipient: its hover says "Enter a recipient address").
- Passes if the 210 names show owned statuses and the batch buttons work without a page reload.

- **Blocks mined:** 0
- **Evidence:** `N1-1-owned`, `N1-3-bar`.
- **Cleanup / end state:** `Clear` the selection.

## N2 — Transfer more than 100 names in one go

- **Priority:** P1
- **Covers:** B3a, B3b, B9
- **Regression of:** a selection over 100 names could not be transferred at all; the confirmation showed `~0.000000 HNS`.
- **Preconditions:** N1; `$WADDR` at hand; the signer unlocked or the passphrase at hand.

Steps:

1. Names → `Owned`. In the batch bar press `Select all <n> transferable`. Read the bar.
2. Type `$WADDR` in `Transfer to hs1q… / rs1q…`, press `Transfer Selected`. While it builds, read the bar.
3. Read the confirmation `Confirm batch transfer` (open `Names (<n>)`).
4. `Confirm`. Confirm every secure window that opens. While it runs, read the bar.
5. Read the toasts and Activity. `scripts/regtest.sh rpc getrawmempool`; for one txid from the toasts, `scripts/regtest.sh rpc getrawtransaction <txid> 1` and count its outputs whose covenant action is `TRANSFER`.
6. `mine 1`, Sync. Open one `NM` name.

Expected:

- Step 1: `<n> selected`; under the buttons "<n> names go out as <k> transactions of up to 100, one confirm for all", with `k` = ⌈n / 100⌉ (210 names → 3).
- Step 2: the bar shows "Building transfer transaction 1 of <k>…" until the confirmation opens.
- Step 3: "You're about to transfer <n> names in <k> transactions, signed and sent one after another after this confirm."; `Estimated fee: ~<f> HNS for all transactions` with `f` **above zero** (about 0.016 HNS per 100 names at the default rate); `Amount: <a> HNS in the first of <k> transactions`; `To: <$WADDR>`.
- Step 4: the bar shows "Signing and sending transfer transaction <i> of <k>…" for i = 1 … k. The confirmation closes after the first transaction.
- Step 5: one toast per transaction, `Batch transfer <i> of <k> broadcast <12 hex>… (<m> name(s))`, with m = 100, 100, …, and the remainder; the mempool holds k transactions; the sampled one carries 100 (or the remainder) `TRANSFER` outputs. The selection is empty afterwards.
- Step 6: the name reads `Transferring · finalize opens in 9 blocks` (see N4).
- Passes if every selected name has a TRANSFER to `$WADDR` after one confirmation, and the fee shown is not zero.

- **Blocks mined:** 1
- **Evidence:** `N2-1-bar`, `N2-2-building`, `N2-3-confirm`, `N2-4-sending`, `N2-5-activity`; the mempool and transaction output.
- **Cleanup / end state:** the names are in transfer to the hsd wallet. Either finalize them in N4, or cancel them (N4's `Cancel transfer` per name is slow: prefer finalizing; they then belong to the hsd wallet and are spent for this library).

## N3 — The names that block a transfer are named and dropped in one click

- **Priority:** P1
- **Covers:** B3b
- **Preconditions:** the seller profile owns at least two names not in transfer (if N2 moved them all, run seed-many-names again or use the setup names SA … SF).

Steps:

1. Open one owned name (role P) → Transfer & sell → transfer it to `$WADDR`; confirm. Do not mine.
2. Names → `Owned` → header checkbox (`Select all names`). Read the bar.
3. Type `$WADDR` in the recipient field; read `Transfer Selected` (hover it).
4. Press `Drop <k> not transferable, keep <m>`. Read the bar and `Transfer Selected` again.
5. `Clear`. Press `Select all <n> transferable`. Read the selection count.

Expected:

- Step 2: the bar lists "Not transferable: <P> (a transfer is already pending)" (or, before a Sync, the capability's own sentence "a transfer is already pending — finalize or cancel it first"); a name still in its auction would read "still in its auction", a lapsed one "expired", an unregistered one "not registered". More than three blocking names end with "and <j> more".
- Step 3: `Transfer Selected` is disabled; its hover repeats the "Not transferable: …" sentence.
- Step 4: the count drops by the number of blocking names; the "Not transferable" line and the Drop button are gone; `Transfer Selected` is enabled.
- Step 5: the count equals the number of owned names minus P.
- Passes if every blocking name is named with a reason and one click removes exactly them.

- **Blocks mined:** 0
- **Evidence:** `N3-2-blockers`, `N3-4-dropped`, `N3-5-select-all`.
- **Cleanup / end state:** `Clear`. P stays in transfer: finalize it in N4 or `Cancel transfer` it there.

## N4 — A name in transfer says it once, with when Finalize opens

- **Priority:** P1
- **Covers:** —
- **Regression of:** the name page repeated "transfer in progress, finalize after the lockup" four times, never said when, and a guided card under the title "Finalize Transfer" said "Records, transfer, selling and signing are in the tabs above; Renew is the blue button."
- **Preconditions:** a name of the seller in transfer, mined (N2 step 6 or N3's P after `mine 1`).

Steps:

1. Names → `Owned`: read the name's row. Open the name. Read the status line under the name, the timeline, every card of Overview, and the header buttons (hover the disabled `Finalize transfer`).
2. Wallet → Your names: read the name's row if it is listed.
3. `mine 9`, Sync. Read the name page again.
4. `Finalize transfer` (or, for N2's names, Names → select them → `Finalize Selected`, a series like N2) → confirm. `mine 1`, Sync.

Expected:

- Step 1: the Names row reads `Transferring · finalize in <b> blocks` (blocks only on regtest; on mainnet `finalize in <b> blocks (~<h>h)`); status line `Transferring · finalize opens in <b> blocks` (regtest shows blocks only, never hours); exactly **one** card, "Transfer in progress", reading "Sent at block <h>. Finalize opens in <b> blocks; then the name leaves this wallet. Until then you can cancel." with `Cancel transfer`; no card titled "Finalize Transfer", no sentence "Renew is the blue button"; no `Auction` card (the name is registered and ours; Details has its price); block numbers with thousands separators.
- Step 2: `Transferring` with `finalize in <b> blocks`, never "Transfer ready" or "finalize it".
- Step 3: status line `Transferring · ready to finalize`; the card reads "The transfer lockup is over: finalize it with the button above and the name leaves this wallet. Until then you can cancel."; `Finalize transfer` enabled.
- Step 4: after the block the name is no longer in the seller's Names.
- Passes if the page says the transfer once, with the countdown, and says "ready" only once Finalize is possible.

- **Blocks mined:** 10
- **Evidence:** `N4-1-transferring`, `N4-2-your-names`, `N4-3-ready`, `N4-4-gone`.
- **Cleanup / end state:** the names belong to the hsd wallet.

## N5 — A lost auction is shown as lost, never as owned

- **Priority:** P1
- **Covers:** —
- **Regression of:** on mainnet a name whose winner never registered read `Owned` in the losing wallet, with every action locked as "owner coin not synced locally"; the losing bid's lockup was not offered for Redeem.
- **Preconditions:** the seller profile with at least 3 HNS spendable; a fresh name L (`ql$TAG`).

Steps:

1. Names → type L in the lookup → `Look up` → `Open auction…` → confirm. `mine 6`.
2. In the app, bid 1 HNS with a 2 HNS lockup on L; confirm. `W sendbid L 11 12`. `mine 6`.
3. Reveal with the hsd wallet **first**: `W sendreveal L`; `mine 1`. Then reveal in the app; confirm; `mine 10`. Nobody registers.
4. Sync. Names: read L's row and the chips. Open L: read the status line and Details.
5. Redeem L's lockup from its row or page; confirm; `mine 1`; Sync.

Expected:

- Step 4: L is **not** under `Owned`; its row reads `You lost · get your 1 HNS back` (the amount may be written `1.000000`) with `Redeem`; the name page's status line reads `You lost · reclaim 1 HNS`; nothing on the page says `Owned` or "owner coin not synced locally".
- Step 5: Activity shows a `Redeem` row; after the block L's row is gone and 1 HNS is back in the balance (less the fee).
- Passes if the losing wallet sees the loss and gets its lockup back. On mainnet the bug came through an explorer read; regtest has no explorer, so this case checks the node's verdict, and unit tests pin the stale-address path.

- **Blocks mined:** 24
- **Evidence:** `N5-4-row`, `N5-4-page`, `N5-5-redeemed`; `info L` after step 3 (owner is the hsd wallet's reveal).
- **Cleanup / end state:** L is won by the hsd wallet and unregistered.

## N6 — A locked wallet is a notice, not an error

- **Priority:** P3
- **Covers:** —
- **Preconditions:** the seller profile; any owned name.

Steps:

1. Lock the wallet (the lock button beside the switcher). Open the owned name.

Expected:

- A yellow (amber) banner "Name actions unavailable. Unlock your wallet to sign transactions." with `Unlock`; not red.
- Passes if the banner uses the warning colours.

- **Blocks mined:** 0
- **Evidence:** `N6-1-banner`.
- **Cleanup / end state:** unlock.

## N7 — A renewal of more than 100 names is a series too

- **Priority:** P3
- **Covers:** B3a
- **Preconditions:** more than 100 owned names of the seller that are renewable (the 210 seeded names, before N2 moves them, or after another seed-many-names).

Steps:

1. Names → `Owned` → `Select all <n> transferable` (or the header checkbox and `Drop …`). `Renew Selected`. Read the confirmation; `Confirm`; confirm every secure window.
2. `mine 1`, Sync. Open one renewed name: read `Renewed` in Details.

Expected:

- Step 1: "You're about to renew <n> names in <k> transactions, …"; a non-zero fee "for all transactions"; one toast `Batch renew <i> of <k> broadcast …` per transaction.
- Step 2: `Renewed` reads the block of step 2, with a thousands separator.
- Passes if every selected name is renewed after one confirmation.

- **Blocks mined:** 1
- **Evidence:** `N7-1-confirm`, `N7-2-renewed`.
- **Cleanup / end state:** the names stay with the seller.
