# cancel-and-bring-home

Cancel a `Listed for sale` (or Restored) listing and finalize the name home.

- **Parameters:** `NAME`, a Listed or Restored row of the active profile.
- **Blocks mined:** 11 (1 + 9 + 1).

## Steps

1. My listings → `Cancel listing` on `NAME` → the dialog `Cancel the listing of .NAME` shows its R28 sentences before anything is signed → `Sign and send the cancel` → secure window `Confirm cancel` → confirm. Toast: "Cancel sent. Finalize the name home once it is mined and the lockup is over." The row reads `Cancelling`.
2. [mine-and-sync](mine-and-sync.md) 1: `Cancelled · finalize to bring it home`, `Finalize home` disabled with "the cancel can be finalized in 9 blocks" (right after the block, before a sync has counted it, the reason may be "the cancel is mined, but its transfer lockup is not counted yet: sync, then finalize it").
3. [mine-and-sync](mine-and-sync.md) 9: `Finalize home` enabled → press it → secure window (action `shakedex_cancel_finalize`) → confirm. Toast: "Sent. The name comes home once the finalize is mined." The row reads `Coming home`.
4. [mine-and-sync](mine-and-sync.md) 1: `Cancelled`, no buttons.

## End state

`NAME` is an ordinary owned name of the active profile (Owned Names row with `Manage`, no listing badge); on chain `info NAME` shows `transfer 0` and the owner at the `Name comes home to` address of step 1's secure window. Another profile of the same phrase that tracked the same lock may need a Sync (or more) to read `Cancelled`.
