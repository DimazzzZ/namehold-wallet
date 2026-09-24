# Batch name operations

Status: implemented. Written after the fact, when a review found the feature
had shipped across backend, frontend and user docs with no spec.

## 1. Summary

Acting on one name at a time is the wrong unit of work for a portfolio: a
hundred names come up for renewal together, and a wallet that bid on twenty
names has twenty reveals to make in the same window. The user selects names in
the Owned Names table, presses one button, sees a confirmation naming the count
and the fee, and signs once. The result is a single transaction with a single
txid — it all lands or none of it does.

## 2. Terms

- **Batch** — one transaction carrying the same covenant for several names.
  Not a queue and not a series: there is one draft, one signature and one txid.
- **Batch size** — how many names one batch carries. Capped at
  `MAX_BATCH_SIZE`.
- **Per-name prefetch** — the `(name, name_hash, owner_coin, on_chain_state)`
  tuple the command resolves for every name before it builds anything, so a
  name that cannot participate stops the batch before any write.
- **Inner builder** — the `build_batch_*_draft_inner` function: the pure half,
  taking a `Connection` and the prefetch rather than a Tauri `State`.

## 3. Requirements

**B1 — Six covenants can be batched.** `bid`, `renew`, `transfer`, `reveal`,
`redeem` and `finalize`. Nothing else is offered in bulk.
*Enforced:* `commands/names.rs::build_batch_{bid,renew,transfer,reveal,redeem,finalize}_draft`.
*Pinned:* the happy path of each — `build_batch_bid_draft_tests::batch_bid_happy_path_persists_summary_commitments_and_draft`,
`build_batch_renew_draft_tests::batch_renew_happy_path_persists_draft`,
`build_batch_transfer_draft_tests::batch_transfer_happy_path_persists_draft`,
`build_batch_reveal_draft_tests::batch_reveal_happy_path_persists_draft`,
`build_batch_redeem_draft_tests::batch_redeem_happy_path_persists_draft`,
`build_batch_finalize_draft_tests::batch_finalize_happy_path_persists_draft`.

**B2 — A batch is one transaction, not a chunked series.** There is no
`createbatch` RPC in hsd and no chunking here: every batch builder calls
`actions::build_batch_plan` once and persists one draft.
*Enforced:* `commands/names.rs` (each `*_inner` → `build_batch_plan` →
`persist_with_conn`).
*Pinned:* the happy-path tests above assert a single persisted draft.

**B3 — At most 100 names per batch.** `MAX_BATCH_SIZE = 100`, refused with the
count and the limit in the message. The number is deliberately well under
hsd's per-transaction covenant limits (300 OPENs, 600 UPDATEs, 600 RENEWs);
per-block limits are the same as per-transaction ones, so a full batch is never
refused for carrying too many covenants.
*Enforced:* `commands/names.rs::MAX_BATCH_SIZE` and the length check in each
batch command.

**B4 — One bad name aborts the whole batch, before any write.** Every name's
owner coin and on-chain state are resolved up front, and a shared destination
is decoded up front, so a name in the wrong phase or an unparseable address
fails while nothing has been persisted or reserved. A partially applied batch
would be worse than none: the user would have to work out which half went.
*Enforced:* the per-name prefetch loop and the early `address::decode` in each
batch command.
*Pinned:* `build_batch_bid_draft_tests::batch_bid_one_bad_phase_aborts_entire_batch`,
`build_batch_finalize_draft_tests::batch_finalize_rejects_non_transfer_coin`,
`build_batch_reveal_draft_tests::batch_reveal_bad_nonce_length_errors`.

**B5 — Coin reservation and draft persistence are atomic.** The inner builder
requires the caller to hold the DB mutex for its whole duration, so the coins a
batch reserves and the draft that claims them are written under one guard. A
second batch cannot select a coin the first has taken.
*Enforced:* each `build_batch_*_draft_inner` (documented on the function), with
the wrapper taking `state.db.lock()` immediately before the call.
*Pinned:* `build_batch_transfer_draft_tests::batch_transfer_happy_path_persists_draft`
asserts the reservation count.

**B6 — A batch transfer names one recipient, and the value stays put.** The
recipient is decoded once and written into every name's TRANSFER covenant; the
output value stays at each name's current owner address until FINALIZE moves
it, as consensus requires.
*Enforced:* `commands/names.rs::build_batch_transfer_draft_inner`.
*Pinned:* `build_batch_transfer_draft_tests::batch_transfer_uses_owner_address_for_output`.

**B7 — A batch of one is still a batch.** Selecting a single name takes the
same path rather than falling back to the singular command, so the two cannot
drift.
*Pinned:* `batch_transfer_single_name`, `batch_renew_single_name`,
`batch_reveal_single_name`, `batch_redeem_single_name`,
`batch_finalize_single_name`.

**B8 — An empty selection is refused where it is meaningless.** `transfer` and
`finalize` reject an empty name list outright. `bid`, `renew`, `reveal` and
`redeem` build a zero-input draft instead, which is what their callers expect
when a filter matches nothing.
*Pinned:* `batch_transfer_empty_errors`, `batch_finalize_empty_errors`,
`batch_bid_empty_specs_persists_zero_output_draft`,
`batch_renew_empty_persists_zero_input_draft`,
`batch_reveal_empty_persists_zero_input_draft`,
`batch_redeem_empty_persists_zero_input_draft`.

**B9 — The confirmation says what will be signed.** `BatchConfirmModal` shows
the count, the estimated fee and a collapsible list of the selected names.
Cancel closes without broadcasting; Confirm signs and broadcasts in one step.
The amount it reports sums every output except change (R13d of the
[multiple-bids spec](./2026-09-20-multiple-bids-per-name.md)), which matters
here because a batch reveal or redeem carries one output per bid.
*Enforced:* `src/components/BatchConfirmModal.tsx`, driven from
`src/components/WalletView.tsx`.
*Pinned:* `src/components/__tests__/batch-transfer.test.tsx`.

**B10 — A batched action still counts as spending the name.** The draft a
batch builder persists records itself as `batch-<action>`, and the owner-spend
check strips that prefix — see R11d of the
[multiple-bids spec](./2026-09-20-multiple-bids-per-name.md).

## 4. Explicitly not enforced

- **No chunking.** A selection over `MAX_BATCH_SIZE` is refused, not split.
  The user reduces the selection.
- **No per-name result.** A batch is one transaction, so there is no notion of
  "eight succeeded, two failed" — the node accepts it or it does not.
- **No cross-covenant batching.** Renewing some names and revealing others in
  one transaction is possible on-chain but not offered; each button carries one
  covenant.
- **No fee negotiation per name.** One fee rate applies to the whole batch.

## 5. Known gaps

- **The 100-name cap is a round number, not a measured limit.** It is well
  under the consensus covenant limits, but the real ceiling is transaction
  size, which depends on the inputs coin selection picks. A batch of 100 names
  with many small inputs could still be large.
- **`build_batch_*_draft` commands carry `coverage(off)`.** They are IO shells,
  which the standard allows, but it means the length and prefetch checks are
  covered only through their inner builders and the frontend tests.

## 6. Pointers

- Backend: `src-tauri/src/commands/names.rs` (`MAX_BATCH_SIZE`, the six
  `build_batch_*_draft` commands and their `*_inner` halves),
  `src-tauri/src/noncustodial/actions.rs` (`build_batch_plan`).
- Frontend: `src/components/BatchConfirmModal.tsx`,
  `src/components/WalletView.tsx` (selection + action bar),
  `src/components/BatchBidModal.tsx`.
- Tests: `src-tauri/src/tests/build_batch_{bid,renew,transfer,reveal,redeem,finalize}_draft_tests.rs`,
  `src/components/__tests__/batch-transfer.test.tsx`.
- Docs: `docs/USER_MANUAL.md` ("Batch operations"), `docs/QA_WORKFLOWS.md`
  (section I).
- CHANGELOG: the batch-transfer entry under `## [Unreleased]`.
