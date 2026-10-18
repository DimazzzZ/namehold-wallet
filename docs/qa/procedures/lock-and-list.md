# lock-and-list

Take an ordinary owned name to `Listed for sale` from the seller profile: Lock, the transfer lockup, Finalize & sign, the FINALIZE mined, and optionally Export.

- **Parameters:** `NAME` (an ordinary owned name of the active profile); `MODE` = Buy Now with `PRICE` (default 5), or Reverse auction with `START`, `FLOOR`, `STEPS`, `MINUTES` (default 5, 3, 3, 15); `EXPORT` = a path for the listing file, or none.
- **Blocks mined:** 11 (1 + 9 + 1).

## Steps

1. Wallet › Owned Names → `Manage` on `NAME` → Ownership → `Sell…`.
2. In the Lock dialog (`Sell .NAME`), pick `Buy Now — one price` or `Reverse auction — the price falls step by step to a floor`. There is no price field here.
3. `Lock name` → the dialog shows a `Network fee` row and `Back` / `Sign and send the lock` → `Sign and send the lock` → the secure window (passphrase first if the signer is locked) → confirm. Toast: "Lock sent. Once the transfer lockup is over, set the price with Finalize & sign in Market › My listings."
4. [mine-and-sync](mine-and-sync.md) 1: Owned Names and Market › My listings both read `Locking · 9 blocks`.
5. [mine-and-sync](mine-and-sync.md) 9: My listings reads `Ready to Finalize & sign`.
6. `Finalize & sign` → Buy Now: `Price (HNS)` = `PRICE`; reverse auction: `Start price (HNS)`, `Floor price (HNS)`, `Number of prices (2 to 1000)`, `Minutes between prices (at least 15)` → `Finalize & sign` → secure window `Confirm Finalize & sign` (one `Price step N` row per price) → confirm. Toast: "Finalize sent. The listing becomes Listed for sale once the finalize is mined." The row reads `Finalizing into the lock`.
7. [mine-and-sync](mine-and-sync.md) 1: the row reads `Listed for sale` with its price line (`Price 5.000000 HNS`, or `Now … HNS · next … HNS in ~T · floor … HNS`).
8. If `EXPORT`: `Export listing file` → native Save panel titled "Save the listing file" (default name `<name>-listing.json`; it is a separate window, possibly of another process) → save to `EXPORT` → toast "Listing file saved". Export never asks to unlock. Check the file: `python3 -c "import json,sys;d=json.load(open(sys.argv[1]));print(d['version'],d['name'],len(d['data']))" EXPORT`.

## End state

`NAME` is `Listed for sale` in the seller's My listings and on its Owned Names row; on chain `info NAME` shows the owner as the FINALIZE into the lock and `transfer 0`; the listing file is at `EXPORT`.
