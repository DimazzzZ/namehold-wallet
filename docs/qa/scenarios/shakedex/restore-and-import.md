---
area: Shakedex restore a lock by name, own listing file import, second device
priority: P1–P3
blocks: ~92
requires:
  - environment.md (seller and second-device profiles of one recovery phrase; names SE, SF)
  - procedures/lock-and-list.md
  - procedures/restore-and-import.md
  - procedures/cancel-and-bring-home.md
  - procedures/switch-profile.md
  - procedures/cli-fill.md
last_verified: "eec6b2d (same tree as 150d58a) for C4; bc8cd4d for R1–R6; 2026-10-18, report kept outside the repo"
---

# Restore and import

A wallet restored from its recovery phrase does not find its locks on its own (spec §5): the seller restores each one by name, and imports its saved listing file to get the prices back. Since R34 the wallet also looks for its locks once by itself: the first sync of a restored profile that ends with the signer unlocked adopts every name still in its lock, so a profile that is unlocked and synced before a case restores by name may already hold the row ([R8](#r8--a-restored-profile-finds-its-own-locks-after-the-first-sync)); keep the signer locked until the restore step. These cases use the second-device profile ([environment](../../environment.md#profiles)) as the restored wallet.

## R1 — Restore by name, import the seller's file, cancel and bring the name home from the second device

- **Priority:** P1
- **Was:** full QA R1; T7 S7; T7 recheck F7
- **Covers:** R17, R21, R28, R32
- **Regression of:** F7 (the second profile refused its own listing file: "payment address is not one of this wallet's addresses"), N1 (Cancel listing after a restore and own-file import failed with "corrupted listing: no cancel address")
- **Preconditions:** SE an ordinary owned name of the seller profile. The second device's signer stays locked until the restore step (see the note under the title).

Steps:

1. Seller: [lock-and-list](../../procedures/lock-and-list.md) SE, Buy Now 5, `EXPORT=$QA_DIR/SE-1.json`. (Run [S1](selling.md#s1--a-listed-names-name-actions-point-to-my-listings) on this row now. Before the `mine 9` of the lock, [R4](#r4--restore-refusals)(c) can be timed here.)
2. [switch-profile](../../procedures/switch-profile.md) to the second device → Market › My listings → in `Restore a lock by name` type SE with an uppercase letter and a space inside (e.g. `QFE <rest>`) → read the hint under the field → `Restore`.
3. `Import its listing file` → **paste** the text of `SE-1.json` → `Import`.
4. `Restore` SE once more ([R4](#r4--restore-refusals)(g)).
5. `Cancel listing` → read the first sentence → `Sign and send the cancel` → read the secure window → confirm.
6. Continue [cancel-and-bring-home](../../procedures/cancel-and-bring-home.md) from its step 2 in this profile.
7. [switch-profile](../../procedures/switch-profile.md) back to the seller, Sync (twice if needed); read My listings and Owned Names; `info SE`.

Expected:

- Step 2: `Restores .<se>` shows under the field (the typed text differs beyond case or a leading dot); toast "Restored .<se>: Locked — listing details unknown"; the row reads `Locked — listing details unknown`, no mode, buttons `Cancel listing` and `Import its listing file`.
- Step 3: toast "Listing file imported: the listing is Listed for sale."; the row reads `Listed for sale`, `Buy now`, `Price 5.000000 HNS`.
- Step 4: refused "'<se>' is already tracked by one of this wallet's listings"; no row added.
- Step 5: the Buy Now first sentence; the `Confirm cancel` secure window opens (no "corrupted listing: no cancel address" before it), `Current price` `5.000000 HNS`, `Name comes home to` an `rs1…` address; after confirming, `Cancelling` and Activity `Cancel listing`.
- Step 6: `Cancelled · finalize to bring it home` with "the cancel can be finalized in 9 blocks", then `Coming home`, then `Cancelled`; SE an ordinary owned name in this profile; `info SE` shows `transfer 0` and the owner at step 5's `Name comes home to` address.
- Step 7: the seller's SE listing ends `Cancelled` (it may pass through `Cancelled · finalize to bring it home` with "the cancel is mined, but its transfer lockup is not counted yet: sync, then finalize it" on the first sync); SE an ordinary owned name. A seller row that stays `Listed for sale` or reads `Sold` is ◐ with spec §5 "One device per listing" named; add `info SE` and the cancel's home address.
- Passes if the second device restores, imports, cancels and brings the name home, and the seller ends `Cancelled`.

- **Blocks mined:** 22
- **Evidence:** screenshots `R1-2-restored`, `R1-3-imported`, `R1-5-secure-cancel`, `R1-6-cancelled`, `R1-7-seller`; `info SE` after step 6.
- **Cleanup / end state:** SE an ordinary owned name of both profiles; both rows `Cancelled`.

## R2 — Sold, seen from a second device that restored the lock by name only

- **Priority:** P1
- **Was:** full QA R2
- **Covers:** R22, R32
- **Preconditions:** SF an ordinary owned name of the seller profile. The second device's signer stays locked until the restore step (see the note under the title).

Steps:

1. Seller: [lock-and-list](../../procedures/lock-and-list.md) SF, Buy Now 5, `EXPORT=$QA_DIR/SF.json`.
2. Second device: [restore-and-import](../../procedures/restore-and-import.md) SF without a file: `Locked — listing details unknown`.
3. Terminal: [cli-fill](../../procedures/cli-fill.md) mined at once with `$QA_DIR/SF.json`.
4. Second device: Sync; read My listings and Owned Names.
5. Seller: [switch-profile](../../procedures/switch-profile.md), Sync; read the same.

Expected:

- The second device's row becomes `Sold`, no buttons (found from the price step's `0x84` signature, with no payment address known here).
- The seller's row reads `Sold` (it may show `Listed for sale` while its first sync runs).
- Neither shows `Listed for sale` once synced. Note in each profile whether Owned Names lists SF and with which label. `info SF` shows the fill's TRANSFER as the owner.
- Passes if both profiles end `Sold`.

- **Blocks mined:** 12
- **Evidence:** screenshots `R2-1-restored`, `R2-2-sold-second-device`, `R2-3-sold-seller`.
- **Cleanup / end state:** SF sold to the hsd wallet.

## R3 — Cancel a lock restored without its file; relisting reuses the lock

- **Priority:** P2
- **Was:** full QA R3
- **Covers:** R21, R28, R32
- **Preconditions:** R1 done (SE home in both profiles). The second device's signer stays locked until the restore step (see the note under the title).

Steps:

1. Seller: [lock-and-list](../../procedures/lock-and-list.md) SE again, Buy Now 5, `EXPORT=$QA_DIR/SE-2.json`.
2. Second device: [restore-and-import](../../procedures/restore-and-import.md) SE without a file (allowed: its old SE listing is `Cancelled`, not open).
3. `Cancel listing` → read the dialog and the secure window → confirm; continue [cancel-and-bring-home](../../procedures/cancel-and-bring-home.md) from its step 2.
4. Compare `SE-1.json` and `SE-2.json` ([S11](selling.md#s11--every-listing-pays-a-fresh-address-relisting-reuses-the-lock)).

Expected:

- The first sentence is "Anyone holding the listing file can still buy the name at its current price until this cancel is mined."; the secure window's `Current price` reads "not known on this device (a restored lock without its listing file)".
- Then `Cancelling` → `Cancelled · finalize to bring it home` → `Coming home` → `Cancelled`; SE owned again in both profiles (the seller may need two syncs).
- The two files share `publicKey`, with different lock outpoints and payment addresses.
- Passes if a lock with no file can still be cancelled and brought home.

- **Blocks mined:** 22
- **Evidence:** screenshots `R3-1-secure-not-known`, `R3-2-cancelled`.
- **Cleanup / end state:** SE an ordinary owned name of both profiles.

## R4 — Restore refusals

- **Priority:** P2
- **Was:** full QA R4
- **Covers:** R32
- **Preconditions:** the second device active unless noted; names in the states below, timed inside other cases.

Steps:

1. Restore each of these and read the line under the field:
   - (a) an ordinary owned name (SC before S4, or X1 after B8);
   - (b) a name the CLI has listed (X2 if not bought, or X4);
   - (c) a name `Locking` in the seller profile (after the lock's `mine 1` in S2 or R1);
   - (d) a name whose cancel is mined but not finalized home (S2 step 7), or one bought and not yet finalized by its buyer;
   - (e) a name that was never registered;
   - (f) in the **seller** profile, a name it has Listed;
   - (g) the same name twice in the second device (R1 step 4).

Expected:

- (a) and (b): "the name is not in this wallet's lock for it: its owner coin is elsewhere".
- (c): "the name is still on its way into the lock: a TRANSFER committing to it is mined but the FINALIZE into it is not; the way out is Cancel transfer, from the wallet that holds the name".
- (d): "the coin in the lock is already a TRANSFER, not a FINALIZE: a cancel awaiting its finalize, or a mined purchase; there is no lock to restore".
- (e): "'<n>' has no on-chain state or has expired: there is no lock to restore".
- (f) and (g): "'<n>' is already tracked by one of this wallet's listings".
- Nothing is added for any of them. Passes if each is refused with its sentence.

- **Blocks mined:** 0
- **Evidence:** screenshot `R4-c-still-locking`; quote the rest.
- **Cleanup / end state:** nothing stored.

## R5 — Own listing file import refusals

- **Priority:** P2
- **Was:** full QA R5
- **Covers:** R2, R32
- **Preconditions:** a Restored row in the second device (SF in R2 before the fill, SE in R3, or SD); the CLI's X1 file; the stale `SE-1.json`.

Steps:

1. `Import its listing file` and paste, one at a time: (a) the CLI's X1 file; (b) on SE in R3, the stale `SE-1.json` of SE's first lock; (c) a copy of the row's right file with `feeAddr` set to `$WADDR` and one step's `fee` set to 50000.

Expected:

- (a) "there is no Restored lock of '<x1>' here: restore it by name first".
- (b) "this listing file is for another lock coin".
- (c) "this listing file names a market fee, which this wallet never writes: it is not this wallet's own listing file".
- The row stays `Locked — listing details unknown`. Passes if each is refused and nothing changes.

- **Blocks mined:** 0
- **Evidence:** screenshots `R5-b-other-lock-coin`, `R5-c-fee-refused`.
- **Cleanup / end state:** the Restored row unchanged.

## R6 — Expired rows

- **Priority:** P3
- **Was:** full QA R6
- **Covers:** R19 (Expired state)
- **Preconditions:** a listing whose name expired in its lock, if one exists from an earlier, taller stretch of the chain. A deliberate expiry is BLOCKED ✗ within budget (5 000 blocks after the FINALIZE into the lock; covered by the live test `shakedex_listed_lock_expires_with_its_name`).

Steps:

1. In both profiles: My listings → find the expired row; Owned Names; Sync.

Expected:

- `Expired in the lock`, no buttons, no price line, in every profile that tracks it (a Restored lock included); not on Owned Names; unchanged by Sync.
- Passes if seen as above; with no expired row on the chain, record BLOCKED ✗.

- **Blocks mined:** 0
- **Evidence:** screenshot `R6-1-expired`.
- **Cleanup / end state:** nothing changed.

## R8 — A restored profile finds its own locks after the first sync

- **Priority:** P2
- **Covers:** R32, R34
- **Preconditions:** SE an ordinary owned name of the seller profile; a second-device profile that has not been synced with its signer unlocked (create it fresh from the same recovery phrase, or use one whose lock search has not run); the node started with `--index-tx` and `--index-address`.

Steps:

1. Seller: [lock-and-list](../../procedures/lock-and-list.md) SE, Buy Now 5, no export.
2. [switch-profile](../../procedures/switch-profile.md) to the second device → Market › My listings → read the line under the list and `Find my locks`; the signer is locked.
3. `Unlock` in the line → passphrase in the secure window → confirm; wait for the next sync to end (or `Sync` once), then read the toast and My listings.
4. `Sync` again; read My listings and the toast area.
5. `Find my locks`.

Expected:

- Step 2: while the signer is locked the line reads "Names this recovery phrase may have locked for sale were found. Unlock the wallet to restore them." with an `Unlock` button (only when the search has a candidate); no SE row yet.
- Step 3: toast "Found 1 name locked for sale · restored in My listings"; the row of SE reads `Locked — listing details unknown`, no mode, buttons `Cancel listing` and `Import its listing file`; the unlock line is gone.
- Step 4: no toast and no second row: the search is done and does not run again.
- Step 5: toast "No locked names found"; still one SE row, unchanged. (A node that failed to answer for an address or a name gives "Some addresses or names could not be read from your node. Try again later." instead; press again once the node answers.)
- Passes if the first sync after the unlock restores SE once, the second sync searches nothing, and the button finds nothing new. On a node without the address index the line reads "Finding your locks needs a node run with --index-tx and --index-address; Restore a lock by name works without them." and no SE row appears (then restore it by name, [R1](#r1--restore-by-name-import-the-sellers-file-cancel-and-bring-the-name-home-from-the-second-device)).

- **Blocks mined:** 11
- **Evidence:** screenshots `R8-1-unlock-line`, `R8-2-found`, `R8-3-nothing-new`.
- **Cleanup / end state:** SE a Restored lock of the second device and a Listed listing of the seller; [R3](#r3--cancel-a-lock-restored-without-its-file-relisting-reuses-the-lock) can continue from here, and R1 from its step 3 (its Restore by name step would be refused as already tracked).

## R9 — A cancel and a sale made by the second device read as from another device

- **Priority:** P2
- **Covers:** R43 (R36–R42, R44 and R45 are not reachable on regtest: [C5](mainnet-absence.md#c5--not-gui-checkable-on-regtest))
- **Preconditions:** (a) at [R1](#r1--restore-by-name-import-the-sellers-file-cancel-and-bring-the-name-home-from-the-second-device) step 7, or [C4](#c4--a-lowered-buy-now-imported-in-a-second-device-stays-buy-now) step 6 (there the seller cancels and the second device reads); (b) SE an ordinary owned name of the seller, home in both profiles, that no later case needs (after C4): the CLI buys it. The second device's signer stays locked until the restore step (see the note under the title).

Steps:

1. (a) After the cancel made in one profile is mined and both profiles have synced, read the SE row in My listings of the profile that did **not** cancel, then of the one that did.
2. (b) Seller: [lock-and-list](../../procedures/lock-and-list.md) SE, Buy Now 5, `EXPORT=$QA_DIR/SE-3.json`.
3. Second device: [restore-and-import](../../procedures/restore-and-import.md) SE with `SE-3.json`, pasted (if the sync already restored SE by itself, [R8](#r8--a-restored-profile-finds-its-own-locks-after-the-first-sync), import the file on that row). `Lower price` → `4` → `Sign the lower price` → `Confirm Lower price` → confirm. `Export listing file` → `$QA_DIR/SE-3-lowered.json`.
4. [cli-fill](../../procedures/cli-fill.md) mined at once with `SE-3-lowered.json` (the CLI takes the cheapest valid step, 4 HNS).
5. Sync in the second device, then [switch-profile](../../procedures/switch-profile.md) to the seller and Sync (twice if needed); read the SE row in both profiles; `info SE`.

Expected:

- Step 1: the row of the profile that did not cancel reads `Cancelled` with the line "Cancelled from another device or profile with your recovery phrase"; the row of the profile that cancelled reads `Cancelled` without that line.
- Step 3: the second device's row reads `Listed for sale`, `Buy now`, `Price 4.000000 HNS`, with `All 2 price steps`; the seller's row still `Price 5.000000 HNS` (no market off mainnet, so nothing tells the seller of the lower step, and nothing marks its row as managed elsewhere).
- Step 5: the seller's row reads `Sold` and "Sold at a price set on another device" (it was paid 4 HNS, a price none of its own steps names); the second device's row reads `Sold` without that line. `info SE` shows the owner a TRANSFER out of the lock, then the CLI's FINALIZE. A seller row that stays `Listed for sale` after two syncs is ◐ with spec §5 "One device per listing" named; add `info SE`.
- Passes if each profile words the other device's cancel or sale as from another device, and its own without.

- **Blocks mined:** 0 for (a); 12 for (b)
- **Evidence:** screenshots `R9-1-cancelled-elsewhere`, `R9-1-cancelled-here`, `R9-3-lowered-on-second-device`, `R9-5-sold-elsewhere`, `R9-5-sold-here`; `info SE` after step 5.
- **Cleanup / end state:** SE sold to the hsd wallet; both rows `Sold`.

## C4 — A lowered Buy Now imported in a second device stays Buy Now

- **Priority:** P2
- **Was:** T7 fixes recheck C4
- **Covers:** R19, R26, R32
- **Regression of:** Q4 (a Buy Now lowered before a block, exported and imported in another profile, read as `Reverse auction`), and its sequel (the same after a block had moved the median time, until the file named its mode)
- **Preconditions:** SE an ordinary owned name of the seller profile, home in both profiles.

Steps:

1. Seller: [lock-and-list](../../procedures/lock-and-list.md) SE, Buy Now 5, no export yet. Read Activity for SE (the `Lock for sale` and `Finalize into lock` rows, mined).
2. [mine-and-sync](../../procedures/mine-and-sync.md) 1 block, so the median time moves; then `Lower price` 5 → 4; confirm.
3. `Export listing file` → `$QA_DIR/SE-lowered.json`; check the file has `"mode": "buyNow"` and that its two entries' `lockTime` differ (if they are equal, no block moved the median time: record it and go on; the import must read `Buy now` either way).
4. Second device: [restore-and-import](../../procedures/restore-and-import.md) SE with `SE-lowered.json`, pasted.
5. Run [R7](profiles.md#r7--switching-profiles-never-shows-another-profiles-listings) now, before step 6, while the two profiles hold different SE rows (between the restore and the import is when they differ most).
6. Seller: [cancel-and-bring-home](../../procedures/cancel-and-bring-home.md) SE. Read Activity for SE. Second device: Sync.

Expected:

- Step 2: the seller's row stays `Buy now`, `Price 4.000000 HNS`, with `All 2 price steps`.
- Step 4: the second device's row reads `Buy now`, `Price 4.000000 HNS`, and no `Now … · next … · floor …` line; the same price, expiry and lock coin as the seller's. `All 2 price steps` shows here too, at the file's lock times; that is correct. `Reverse auction` here is ❌ (Q4).
- Step 6: Activity shows `Cancel listing` and `Finalize cancel` with Amount 0 and the `⤷` line. The second device's row should end `Cancelled` after a sync; if it stays `Listed for sale` because the cancel's home address lies more than `address_gap_limit` addresses past the last one used on chain (the seller's unfunded reservations count as used only on the seller's device), that is the known gap of spec §5 (◐, not a finding).
- Passes if the imported lowered Buy Now reads `Buy now` with its one current price.

- **Blocks mined:** 23
- **Evidence:** screenshots `C4-1-seller-buy-now-4`, `C4-2-imported-buy-now-4`, `C4-3-steps-expanded`; both lock times from `SE-lowered.json`.
- **Cleanup / end state:** SE an ordinary owned name of the seller; the seller's row `Cancelled`.
