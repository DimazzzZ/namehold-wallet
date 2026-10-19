# restore-and-import

From the second-device profile, adopt a lock the seller made, by its name, and optionally upgrade it with the seller's listing file.

- **Parameters:** `NAME`, Listed in the seller profile; `FILE`, the seller's export of it (fresh: re-export after any Lower price), or none; `HOW` = paste or file.
- **Blocks mined:** 0.
- **Requires:** [switch-profile](switch-profile.md) to the second device.

## Steps

1. Market › My listings → `Restore a lock by name` → type `NAME` → `Restore` (passphrase first if the signer is locked: the lock key is derived from the phrase). Toast "Restored .NAME: Locked — listing details unknown". The row reads `Locked — listing details unknown`, with no mode, and the buttons `Cancel listing` and `Import its listing file`.
2. If `FILE`: `Import its listing file` → the dialog `Import the listing file of .NAME` → paste the file's text into "Paste the listing file this wallet saved for this lock", or `choose a file…` and pick it in the native Open panel → `Import`. Toast "Listing file imported: the listing is Listed for sale." The row reads `Listed for sale` with the seller's price.

## End state

The second device tracks `NAME` as a Restored lock, or as a full Listed listing after step 2. The seller profile is unchanged. Restoring the same name again in this profile is refused as already tracked.
