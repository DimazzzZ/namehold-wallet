---
area: Shakedex gates (profile kind, sending, experimental flag, locked signer) and input refusals
priority: P1–P3
blocks: ~2
requires:
  - environment.md (seller and watch-only profiles)
  - procedures/lock-and-list.md (Ready and Locking rows to refuse on)
  - procedures/switch-profile.md
last_verified: "bc8cd4d, 2026-10-18, report kept outside the repo"
---

# Gates and refusals

What may sign, and what the backend refuses before anything is signed. A gate refusal is the backend's own sentence; the UI shows it on a disabled button (as a hover hint or under a row) or in an alert.

## G1 — Sending disabled: every signing entry point is gated

- **Priority:** P1
- **Was:** full QA G1; T7 S8.2
- **Covers:** R6, R29
- **Preconditions:** the seller profile with an ordinary owned name, a Listed row, an imported buyable row on Browse, and if possible a Restored row (second device) and a purchase `Ready to finalize`.

Steps:

1. Settings → Connections → untick `Allow sending via remote node` → `Save settings`. Note the original value first.
2. (a) Owned Names → `Manage` on the ordinary name → `Sell…`; (b) Market › Browse; (c) My listings; (d) `Restore a lock by name` with a name typed; (e) on a Restored row, `Import its listing file`; (f) on a purchase `Ready to finalize`, `Finalize` in Owned Names and in Market › Your purchases.
3. Tick it back → `Save settings`; check every item again.

Expected:

- (a) `Sell…` disabled, hover "Shakedex needs a local node, or a remote node with sending allowed".
- (b) An imported buyable row still verifies (`✓ Verified`); selected, its panel's line above the button reads "Shakedex needs a local node, or a remote node with sending allowed." and the button `Buy .<name>` is disabled.
- (c) Every signing button (Lower price, Cancel listing, Finalize & sign, Finalize home) disabled with the sentence printed under the row; `Export listing file` stays enabled.
- (d) `Restore` disabled, the sentence under the section.
- (e) `Import its listing file` disabled with the sentence (if the dialog opens instead, `choose a file…` and `Import` are disabled with it).
- (f) `Finalize` disabled with the sentence, in Owned Names as a hover and in Your purchases printed above its rows.
- After step 3 everything is enabled again. Nothing refuses with another reason.
- Passes if every entry point is gated by that sentence and released after; unreadable hovers are ◐.

- **Blocks mined:** 0
- **Evidence:** screenshots `G1-1-sell-disabled`, `G1-2-browse-panel-reason`, `G1-3-listing-restore-disabled`, `G1-4-purchase-finalize-disabled`, `G1-5-restored-import-disabled`.
- **Cleanup / end state:** `Allow sending via remote node` back at its original value (ticked).

## G2 — A watch-only profile cannot buy or sell

- **Priority:** P2
- **Was:** full QA G2; T7 S8.1
- **Covers:** R16, R29
- **Preconditions:** the watch-only profile; a CLI listing file.

Steps:

1. [switch-profile](../../procedures/switch-profile.md) to the watch-only profile.
2. Market › Browse → import the CLI file.
3. Market › My listings: read `Restore a lock by name`.
4. Auctions → look up an ordinary name the seller owns → its actions.
5. Optional (P3): if Add wallet offers an extended-private-key import on regtest, create a throwaway profile and repeat steps 2 and 4; delete nothing.

Expected:

- Step 2: the import verifies as for any profile (a buyable listing joins the list `✓ Verified`; any other verdict is said in the import dialog); selected, the panel's line above the button reads "Shakedex works with a recovery-phrase wallet for now." and the button `Buy .<name>` is disabled.
- Step 3: `Restore` disabled with the same sentence.
- Step 4: `Sell…` disabled with that sentence, if the dialog shows owner actions at all (a watch-only profile may show only Name details: a known issue, not reported again).
- Step 5: `Buy` and `Sell…` disabled with the same sentence.
- Ledger profiles: BLOCKED ✗ (no device); covered by the vitest suites for the gates.
- Passes if no watch-only path can buy, sell or restore.

- **Blocks mined:** 0
- **Evidence:** screenshot `G2-1-watch-only-buy`.
- **Cleanup / end state:** switch back to the seller profile.

## G3 — The mainnet experimental flag does nothing on regtest

- **Priority:** P2
- **Was:** full QA G3
- **Covers:** R15, R29
- **Preconditions:** the seller profile.

Steps:

1. Settings → section `Shakedex`: read the checkbox and its note; note its current value.
2. Untick it (or tick it) → `Save settings`; on Browse import a CLI file and open a name's `Sell…`.
3. Flip it the other way → `Save settings`; the same.

Expected:

- The checkbox reads "Allow buying and selling names on mainnet through Shakedex (experimental)" with the note "Testnet and regtest are always allowed. Finalizing a purchase, and Finalize & sign, Cancel and Lower price on a listing you already have, work whatever this says."
- With either value Browse shows no "Buying on mainnet is off." banner, the imported row's panel offers an enabled `Buy .<name> for <total> HNS` (never `Turn on buying`), and `Sell…` opens the Lock dialog; "Selling names through Shakedex on mainnet is experimental: enable it in Settings" appears nowhere.
- Passes if the flag changes nothing on regtest. Its mainnet refusals are BLOCKED ✗ ([mainnet-absence](mainnet-absence.md)).

- **Blocks mined:** 0
- **Evidence:** screenshot `G3-1-settings`.
- **Cleanup / end state:** the flag at its original value.

## S8 — Price and schedule refusals in Finalize & sign

- **Priority:** P3
- **Was:** full QA S8; T7 S3.3–S3.6
- **Covers:** R19, R24
- **Preconditions:** a `Ready to Finalize & sign` Buy Now row and a Ready reverse-auction row (inside S2 and S5, before their Finalize & sign).

Steps:

1. Buy Now row: `Finalize & sign` → type each into `Price (HNS)`: `0`, `0.0001`, `1.0000001`, `abc`.
2. Reverse-auction row: `Finalize & sign` → leave one field empty; then try each: `2.5` prices; `0x10` prices; start 5, floor 3, 3 prices, `10` minutes; start 5, floor 5; `1` price; 2 prices with a very long interval (more than a year apart in total).
3. Enter a valid schedule (5, 3, 3, 15), then `Close` without signing (or carry on with the case it belongs to).

Expected (each in red under the fields, the submit button disabled, unless noted):

- `price "0": a price must be above 0`; `price "0.0001": below the dust limit of 0.001000 HNS`; `price "1.0000001": HNS has at most 6 decimals`; `price "abc": not a number of HNS`.
- An empty field: "a reverse auction needs its schedule: a start price, a floor, the number of steps and the time between them".
- `2.5`: "reverse auction: 2.5 steps: a schedule has 2 to 1000 steps". `0x10` is treated as incomplete (not as 16).
- 10 minutes: "reverse auction: steps 600 s apart: steps must be at least 15 minutes apart". Floor = start: "reverse auction: the floor 5.000000 HNS is not below the start 5.000000 HNS". One price: "reverse auction: 1 steps: a schedule has 2 to 1000 steps".
- More than a year: the **warning** "The last price comes more than a year after the first, past the listing file's expiry date.", with submit still enabled.
- The valid schedule enables submit and shows its one-line summary.
- Passes if every bad input is refused in these words before anything is signed.

- **Blocks mined:** 0
- **Evidence:** screenshots `S8-1-price-refusal`, `S8-2-schedule-refusal`, `S8-3-year-warning`.
- **Cleanup / end state:** nothing signed.

## S12 — Lock refusals

- **Priority:** P2
- **Was:** full QA S12
- **Covers:** R19, R27
- **Preconditions:** an ordinary owned name; a `Locking` name.

Steps:

1. On the ordinary name: Ownership → `Transfer` to a new hsd wallet address (`W getnewaddress`) → sign and send → `mine 1`, Sync → open its actions; hover `Sell…` (and press it if it is enabled, then `Lock name`).
2. `Cancel transfer` it → sign and send → `mine 1`, Sync (don't lose the name).
3. On the Locking name: open its actions; hover `Sell…`.

Expected:

- Step 1: `Sell…` disabled with the transfer reason on hover; if the dialog opens instead, `Lock name` refuses "a transfer of '<n>' is pending: cancel or finalize it before locking the name".
- Step 2: the name ordinary again, `Sell…` enabled.
- Step 3: `Sell…` disabled with `this name is locked for sale through Shakedex` (or "'<n>' is already locked for sale" if the dialog opens).
- Passes if no name with a pending transfer or lock can be locked again.

- **Blocks mined:** 2
- **Evidence:** screenshots `S12-1-transfer-sell-disabled`, `S12-2-owned-after-cancel`.
- **Cleanup / end state:** the name ordinary and owned.

## X4 — A locked signer for every Shakedex action

- **Priority:** P2
- **Was:** full QA X4
- **Covers:** R19, R26, R28, R32
- **Preconditions:** fold it into the other cases: before each action below press `Lock` on the Wallet page.

Steps:

1. With the signer locked, start each once: `Lock name`, `Finalize & sign`, `Lower price`, `Cancel listing`, `Finalize home`, `Restore`, `Import its listing file`, `Buy`, a purchase's `Finalize`, and `Export listing file`.

Expected:

- Each signing action, and `Restore` and the own-file import (they derive the lock key), first opens the secure window `Wallet passphrase`; after it the action's own secure window opens and completes (or declines cleanly).
- `Export listing file` does **not** ask to unlock; it opens the native Save panel at once.
- A signing action that opens its own secure window with no passphrase prompt while the Wallet page said `Signer locked` is recorded as an observation (it may be the known signer-state quirk after a profile switch), not a finding by itself.
- Passes if every signing action asks to unlock first and Export never does.

- **Blocks mined:** 0 extra
- **Evidence:** screenshots of one passphrase prompt and the following secure window per action family (`X4-buy-unlock-first`, …).
- **Cleanup / end state:** the signer locked or unlocked as the next case needs.
