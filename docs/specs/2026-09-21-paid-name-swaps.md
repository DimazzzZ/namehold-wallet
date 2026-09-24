# Paid name swaps

**Status: not implemented. The UI entry points were withdrawn on
2026-09-21; the backend commands remain, and an offer already recorded can
still be claimed.**

## 1. Summary

Selling a Handshake name for HNS should be one transaction: the buyer pays and
the name moves, or neither happens. The wallet shipped two buttons — "Sell with
payment" and "Buy with payment" — and a seller-side offer record, and the shape
they implement cannot do that. This spec says what was there, why it could not
work, and what a real implementation needs, so the next attempt starts from the
consensus rules rather than from the buttons.

## 2. Terms

- **Seller** — holds the name and its owner coin.
- **Buyer** — pays HNS and should end up holding the name.
- **TRANSFER coin** — the output a TRANSFER covenant creates. It carries the
  recipient inside the covenant (`items[2]` = address version, `items[3]` =
  address hash) and **stays at the seller's own address**: hsd requires
  REGISTER → TRANSFER to keep the address (`rules.verifyCovenants`).
- **Atomic** — one transaction that either performs both halves or is invalid.

## 3. Why the withdrawn shape could not work

**W1 — Only the seller can finalize.** FINALIZE spends the TRANSFER coin, and
that coin sits at the seller's address, so the seller signs it. The wallet
agrees: `build_finalize_with_payment_draft` resolves the owner coin through
`owner_coin_and_state` and fails without it. So the button labelled "Buy with
payment" could only ever be pressed by the party selling — who has nobody to
pay.

**W2 — Nothing was atomic.** Every input in `noncustodial::actions` is signed
`sighash::ALL`. A counterparty cannot add inputs or outputs to a finished
transaction under that flag, so "finalize and pay in one transaction" means one
wallet funding both halves out of its own coins. There is no exchange.

**W3 — The claim verifies less than it says.** `claim_paid_transfer` documents
itself as checking "a P2WPKH output to the seller's address with value >=
price". `find_payment_output` works by exclusion instead: any output **not** at
the buyer's address, worth at least the price, counts. It cannot check the
seller's address because `paid_swap_offers` never records one. A payment to any
third party satisfies it. Not exploitable on its own — the seller supplies the
txid — but "verified" overstates the evidence.

## 4. What a real implementation needs

**R1 — Swap sighash flags.** The seller pre-signs the FINALIZE with a sighash
type that leaves room for the buyer to add their payment: this is what
Shakedex does (`SINGLEREVERSE` + `ANYONECANPAY`). `noncustodial::tx::sighash`
would need those variants, and the signer would need to be willing to produce
them — a wallet that signs `ANYONECANPAY` is signing something a stranger can
complete, which is a decision to make deliberately, not a flag to add quietly.

**R2 — An offer is a signed artefact, not a DB row.** What the seller publishes
must be the pre-signed input plus the price, so a buyer can verify and complete
it without trusting the seller's wallet. The current `paid_swap_offers` table
is local bookkeeping and cannot travel.

**R3 — The seller's payout address is part of the offer.** Without it no check
can answer "was I paid" (W3).

**R4 — The buyer's side is a fill, not a finalize.** The buyer takes the
seller's pre-signed transaction, adds funding inputs and the payment output,
and broadcasts. There is no separate "finalize with payment" command for them.

## 5. Explicitly not enforced

- Nothing stops a seller and buyer arranging payment off-chain and using a
  plain Transfer + Finalize. That works today and is what the wallet supports.
- Removing the UI does not remove the backend commands
  (`create_paid_swap_offer`, `claim_paid_transfer`,
  `build_finalize_with_payment_draft`). They stay so an offer recorded before
  this change can still be claimed through `PaidSwapClaim`, which renders only
  when one exists.

## 6. Pointers

- `src/components/name-actions/OwnershipActions.tsx` — where the two buttons
  and their forms were.
- `src/components/name-actions/PaidSwapClaim.tsx` — the claim panel, kept.
- `src-tauri/src/commands/paid_swaps.rs` — offer records and `find_payment_output`.
- `src-tauri/src/commands/names.rs::build_finalize_with_payment_draft` — W1.
- `src-tauri/src/noncustodial/actions.rs` — the `sighash::ALL` of W2.
- `name-modal-sections.test.tsx :: offers no way to start a paid swap` — pins
  the withdrawal.
- User-facing copy that described the withdrawn flows, corrected to match:
  `docs/USER_MANUAL.md` ("Paid name swaps", and the owned-name action table),
  `README.md` (feature list), `docs/QA_WORKFLOWS.md` (section J). Any future
  attempt has to update these three in the same PR as the code.
