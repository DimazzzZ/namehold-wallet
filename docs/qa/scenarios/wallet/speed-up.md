---
area: Speed up (child pays for parent) of an ordinary send
priority: P1–P2
blocks: ~3
requires:
  - environment.md (seller and watch-only profiles; the seller funded)
  - procedures/mine-and-sync.md
  - procedures/switch-profile.md
last_verified: "never run"
---

# Speed up

Speed up builds a second transaction (the child) that spends the change of a sent, unconfirmed transaction of ours (the parent) and pays the fee for both ([spec](../../../specs/2026-10-21-speed-up-unconfirmed-sends.md); `Covers` below names that spec's R-numbers). Where the button is and what its form and secure window show is in [ui](../../ui.md#speed-up).

The shared regtest node never fills a block, so a parent is mined in the next block whether or not it has a child: these cases check the flow, the numbers shown, the refusals and the tracking, never the miner's ordering. That a child pulls a stuck parent into a full block is pinned by the live tests on a capped node ([C5](../shakedex/mainnet-absence.md#c5--not-gui-checkable-on-regtest)). The Speed up of a Shakedex purchase is [B20](../shakedex/buying.md#b20--speed-up-an-unconfirmed-purchase).

Never `mine` while a case below waits for the parent to stay unconfirmed: a block (from this case, or from another case run alongside) takes the parent and ends the case early. `W`, `WADDR` and `mine` are the helpers of [environment](../../environment.md#regtest-node-and-hsd-wallet).

## U1 — Speed up an ordinary send

- **Priority:** P1
- **Covers:** R1, R2, R7, R8, R9, R10, R11, R16
- **Preconditions:** the seller profile active, sending allowed, the signer unlocked or the passphrase at hand, at least 2 HNS spendable; no unconfirmed send of the seller in Activity; Settings › `Fee rate (doos/kvB)` empty (note its value and restore it at the end), since the send's rate, the `1` refusal and the empty-field answer of step 3 depend on it.

Steps:

1. Wallet → `Send HNS` → `Destination Address` `$WADDR` → `Amount (HNS)` `1` → `Review`; note `Fee` and `Change` → `Sign & Broadcast` → secure window → confirm. Do not mine.
2. Activity: find the `Send` row; read its status and its `Speed up` button (hover it). Press **Sync** on the Wallet page, come back and read the button again.
3. `Speed up` → read the dialog with `Fee rate (doos/vB)` left empty.
4. Type in `Fee rate (doos/vB)`, one after the other, reading the dialog after each: `1.5`, `1`, `100000`, then `10`.
5. With `10`: read every row → `Review and sign` → the secure window (passphrase first if asked) → read every row → confirm.
6. Without mining: read Activity (both rows, the parent's `Speed up` hovered); `scripts/regtest.sh rpc getrawmempool`; `scripts/regtest.sh rpc getmempoolentry <parent txid>`.
7. `mine 1`, Sync; read Activity; `scripts/regtest.sh rpc getrawmempool`.

Expected:

- Step 2: the row is labelled `Send` with status `Pending` and a secondary `Speed up` button. Before the Sync it may be disabled with "Its change is not in your wallet yet; refresh your wallet and try again."; after the Sync it is enabled, with the hover "Pay a higher fee for it through a second transaction".
- Step 3: the dialog `Speed up` opens with "A second transaction spends this one's change and pays the fee for both, so a miner takes them together. The fee goes to the network only; nobody else is paid. The original keeps its transaction id.", the field `Fee rate (doos/vB)` with the placeholder "your node's estimate", and `Cancel` and `Review and sign`. With the field empty it shows one of (quote which): the rows of step 5 with `Target` `<n> doos/vB (your node's estimate)` or `(your fee rate setting)`; or in red "Your node has no fee estimate yet, and this transaction already pays the minimum rate; choose a higher one." (the usual regtest answer: the node has no estimate and the setting is the minimum rate the send already pays); or in red "Your transaction already pays this rate or more; choose a higher one." (an estimate at or below what it pays). With a red sentence `Review and sign` is disabled.
- Step 4: `1.5`: "Enter a whole number of doos per vbyte." and `Review and sign` disabled. `1`: "Your transaction already pays this rate or more; choose a higher one." `100000`: "This would pay more in fee than your node accepts from one transaction (about <cap> HNS); choose a lower rate.", the cap a little above `1.000000 HNS` (hsd's absurd-fee cap for a child of about 108 vbytes). `10`: the rows, no red sentence, `Review and sign` enabled.
- Step 5, the dialog's rows: `This transaction` `<n> vbytes, fee <fee> HNS` with step 1's `Fee`; `Fee rate now` about `1.00 doos/vB` (what the send paid); `Target` `10 doos/vB (the rate you chose)`; `Speed-up fee` `<f> HNS`; `Fee rate with it` `10.00 doos/vB` or a little above, never below; `Change left` equal to step 1's `Change` minus the speed-up fee; `Mempool deadline` about 72 hours after step 1, in local time; then the amber "Speeding up does not extend the 72 hours your node keeps the original transaction: if it is not mined by <date>, both are dropped and nothing is paid by either."
- Step 5, the secure window `Confirm transaction`: `Action` `Speed up`; `Speeds up` the parent's full txid (its first characters are the `Send` row's txid); `Fee` the speed-up fee; `Fee rate` "<now> now, <after> with this fee (target 10 doos/vB, the rate you chose)"; `Paid to` "The network only: nobody else is paid."; `Back to your wallet` the dialog's `Change left`; `Original transaction` "It keeps its transaction id."; `Deadline` the same 72-hour sentence with the date written `YYYY-MM-DD HH:MM UTC`; `Txid`. After confirming: the toast `Speed up sent <first 12 hex of the child's txid>…` and the dialog closes.
- Step 6: a new row labelled `Speed up`, status `Pending`, with "Speeds up <first 10 hex of the parent's txid>…" under it and no `Speed up` button of its own; the `Send` row keeps its txid, reads "Sped up by <first 10 hex of the child's txid>…", and its `Speed up` is disabled with the hover "This transaction is already being sped up." (a second Speed up of the same parent is refused). The mempool holds both txids; the parent's entry shows `descendantcount` 1 and a `descendantsize` above its `size` by the child's size.
- Step 7: both rows `Confirmed` in the same block, with no `Speed up` on either; the parent's txid is the one of step 2; the mempool holds neither.
- Passes if the child pays the fee shown, back to our own change, the parent keeps its txid, the rows link each other, and both confirm. A missed toast or an unreadable hover is ◐.

- **Blocks mined:** 1
- **Evidence:** screenshots `U1-2-speed-up-offered`, `U1-4-refusals` (one per typed rate), `U1-5-form`, `U1-5-secure`, `U1-6-activity-linked`, `U1-7-confirmed`; the `getmempoolentry` output.
- **Cleanup / end state:** 1 HNS moved to the hsd wallet; nothing unconfirmed.

## U2 — An unsent child, a declined child, and a mined parent

- **Priority:** P2
- **Covers:** R1, R3, R14, R16
- **Preconditions:** as [U1](#u1--speed-up-an-ordinary-send).

Steps:

1. Send 1 HNS to `$WADDR` as in U1 step 1; Sync. Activity → the `Send` row's `Speed up` → type `10` → `Review and sign`. When the secure window `Confirm transaction` opens, do not confirm: quit the app (`osascript -e 'tell application "Namehold" to quit'`, [ui](../../ui.md#known-quirks-and-their-workarounds)).
2. Relaunch, unlock if asked; Activity: read the `Send` row (hover its `Speed up`) and any `Speed up` row.
3. On the `Speed up` row press `Discard` (confirm if asked); within 30 seconds read the `Send` row's `Speed up` again.
4. `Speed up` → type `10` → `Review and sign` → in the secure window press `Cancel`. Read the toast and Activity.
5. `mine 1`; do not Sync. Within 30 seconds hover the `Send` row's `Speed up` (or open it: the dialog shows the same refusal in red). Then Sync and read the row.

Expected:

- Step 2: the child built in step 1 survives as an unsent draft: a row labelled `Speed up` offering `Sign & broadcast` and `Discard`; the `Send` row's `Speed up` is disabled with "This transaction already has a Speed up that was never sent; discard it in Activity (its row offers Discard) to speed this one up again." If no `Speed up` row survived the quit, record what you saw and go on from step 4.
- Step 3: toast "Draft discarded"; the `Speed up` row goes; the `Send` row's `Speed up` is enabled again.
- Step 4: the toast "Not signed: Confirmation declined"; no `Speed up` row stays in Activity (declining discards the child); the `Send` row's `Speed up` is still enabled.
- Step 5: before the Sync, `Speed up` is disabled with "This transaction is not waiting in your node's mempool, so a child could not pull it into a block. If it was not mined, send it again instead."; after the Sync the row reads `Confirmed` and has no `Speed up`.
- Passes if an unsent child blocks a second one only until it is discarded, a declined child leaves nothing behind, and a mined parent is never offered a child.

- **Blocks mined:** 1
- **Evidence:** screenshots `U2-2-unsent-child`, `U2-3-offered-again`, `U2-4-declined`, `U2-5-mined-refused`.
- **Cleanup / end state:** 1 HNS moved to the hsd wallet; no draft left.

## U3 — A send that waits on another unconfirmed send; the watch-only profile

- **Priority:** P2
- **Covers:** R2, R5, R13, R16
- **Preconditions:** as [U1](#u1--speed-up-an-ordinary-send); the watch-only profile ([environment](../../environment.md#profiles)). The case sends the whole spendable balance to the seller's own address, so run no other case that spends from the seller until its step 5.

Steps:

1. Wallet: note the **Receive Address** (`rs1…`). `Send HNS` → `Destination Address` that address → `Max` (it builds the send and shows its review) → `Sign & Broadcast` → confirm. Do not mine. Call it A.
2. Sync. `Send HNS` → `$WADDR`, `1` → `Review` → `Sign & Broadcast` → confirm. Do not mine. Call it B. `scripts/regtest.sh rpc getmempoolentry <B's txid>`.
3. Sync; Activity: hover the `Speed up` of A's row and of B's row.
4. [switch-profile](../../procedures/switch-profile.md) to the watch-only profile, Sync; Activity: look for a `Speed up` button on any row. Switch back to the seller.
5. `mine 1`, Sync; read both rows.

Expected:

- Step 2: B builds from A's output, the only coin left (if `Send HNS` is disabled or B is refused for funds, the wallet does not count A's unconfirmed output yet: record it, ◐, and go to step 5); B's mempool entry lists A's txid in `depends`, with `ancestorcount` 1.
- Step 3: A's `Speed up` is disabled with "This transaction has no change to spend, so it cannot be sped up." (`Max` leaves no change); B's is disabled with "This transaction waits on another unconfirmed transaction of the mempool; speed that one up, or wait for it to confirm."
- Step 4: no row of the watch-only profile offers `Speed up` (it has no sent drafts of its own; the backend's "This wallet is watch-only and cannot sign, so it cannot speed up a transaction." is pinned by `a_watch_only_profile_is_refused`, not reachable from the GUI).
- Step 5: A and B both `Confirmed`, no `Speed up` on either.
- Ledger profiles: BLOCKED ✗ (no device). The backend refuses no Ledger profile: its rows offer `Speed up` as the seller's do and the child is signed on the device (spec R13, never signed on a device yet); see [C5](../shakedex/mainnet-absence.md#c5--not-gui-checkable-on-regtest).
- If `Max` is refused as more coins than a standard transaction can carry ("this would spend N coins in one transaction, …"), the seller holds too many small coins for this case: BLOCKED ✗ with the sentence.
- Passes if neither A nor B is offered a child, each with its own reason, and the watch-only profile is never offered one.

- **Blocks mined:** 1
- **Evidence:** screenshots `U3-3-a-no-change`, `U3-3-b-has-ancestor`, `U3-4-watch-only`; B's `getmempoolentry` output.
- **Cleanup / end state:** the seller holds its balance less two fees and 1 HNS; the seller profile active.
