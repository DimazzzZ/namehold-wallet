# buy-a-cli-listing

The seller profile buys a CLI listing file through the Market page, and finalizes the purchased name.

- **Parameters:** `FILE`, a CLI listing file ([cli-list-a-name](cli-list-a-name.md)); `FINALIZE` = yes or no.
- **Blocks mined:** 1 for the purchase; 10 more to finalize (9 + 1).

## Steps

1. Market › Browse → `Import listing…` → in `Import a listing`, `Choose a file…` (native Open panel) → `FILE`, or paste its text into the box → `Import`. The dialog closes with the toast ".<name> imported and verified"; the row joins the list with the tag `From file` and `✓ Verified`, selected. Imported rows add up and are cleared when you leave the Market page.
2. In the panel beside the list, `Buy .<name> for <total> HNS` → the secure window (passphrase first if locked; rows `Action` Purchase, `Name`, `Price (to seller)`, `Network fee`, `Total`, `Paid to`, `Name becomes yours`, `Txid`) → confirm. Toast "Purchase of .<name> sent"; the panel reads "Purchase sent", and Market › Your purchases lists the name. Owned Names shows the name as plain text with the badge `Unconfirmed purchase`.
3. [mine-and-sync](mine-and-sync.md) 1: `Awaiting finalize · 9 blocks`.
4. If `FINALIZE`: [mine-and-sync](mine-and-sync.md) 9 → `Ready to finalize` with a `Finalize` button on Owned Names → `Finalize` → secure window (`Finalize purchased name`) → confirm → toasts "Finalize broadcast — <first 12 hex of the txid>…" and "The name still carries the seller's DNS records: update them once it is yours." → the row reads `Finalize · waiting for a block` → [mine-and-sync](mine-and-sync.md) 1.

## End state

Without finalize: the name is `Awaiting finalize · N blocks` or `Ready to finalize` on Owned Names. With finalize: an ordinary owned name (a link, a checkbox, `Manage`), still carrying the seller's TXT record `shakedex-cli-sell`; `info <name>` shows our FINALIZE as the owner and `transfer 0`.
