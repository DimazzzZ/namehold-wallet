---
area: Shakedex mainnet-only behaviour, checked on regtest by its absence
priority: P1–P3
blocks: 0
requires:
  - environment.md
  - the Lock dialog and My listings rows seen in the selling cases
last_verified: "bc8cd4d for M1 and M2; eec6b2d (same tree as 150d58a) for C5; 2026-10-18, report kept outside the repo"
---

# Mainnet-only parts

LearnHNS Market is mainnet only, and a run never selects a mainnet profile ([rules](../../rules.md)). On regtest these parts are checked by their absence; what cannot be checked at all is BLOCKED ✗ by design, with what covers it instead.

## M1 — Mainnet-only UI is absent on regtest

- **Priority:** P1
- **Was:** full QA M1
- **Covers:** R5, R8, R23, R25, R28, R29
- **Preconditions:** collect it while running S2, B1 and the other listing cases.

Steps:

1. Read the Lock dialog, every My listings row in every state reached, every Cancel dialog, and Market › Browse.

Expected:

- The Lock dialog has no `Publish on LearnHNS Market` checkbox and no publish privacy note, and shows `Shared as a file only: LearnHNS Market lists mainnet names only`.
- No My listings row offers `Publish on LearnHNS Market`, at any state; every row's status line is `Shared as a file only: LearnHNS Market lists mainnet names only`, never "On LearnHNS Market", "Not on market — retrying", "Goes on LearnHNS Market at the next sync" or another market status.
- No Cancel dialog or cancel secure window shows "Told the listing is cancelled…".
- Browse lists no LearnHNS Market listing, has no `Load more`, and the link import is blocked ([B1](buying.md#b1--browse-on-regtest)).
- Passes if all are absent.

- **Blocks mined:** 0
- **Evidence:** screenshots `M1-1-lock-dialog`, `M1-2-row-status`.
- **Cleanup / end state:** nothing changed.

## M2 — No connection to the market's host

- **Priority:** P3
- **Was:** full QA M2
- **Covers:** R23, R25
- **Preconditions:** Browse open, and a Sync running with a Listed listing.

Steps:

1. `lsof -nP -i -a -p $(pgrep -x namehold-wallet)`, and `-p $(pgrep -x namehold-syncd)` if it runs; take three samples.
2. `dig +short market.learnhns.com`.

Expected:

- Connections to `127.0.0.1:14037` (and possibly the update check), none to the market's addresses.
- Passes if no sample shows the market host. This is a sampled check, not a continuous capture: say so in the report.

- **Blocks mined:** 0
- **Evidence:** the `lsof` samples (addresses only).
- **Cleanup / end state:** nothing changed.

## M3 — Mainnet-only selling UI in the browser mock

- **Priority:** P3
- **Was:** QA_WORKFLOWS §J (the mainnet-only and browser QA items)
- **Covers:** R23, R25, R28
- **Preconditions:** mode C ([environment](../../environment.md#modes)): `pnpm dev`, whose mock profile is mainnet and reaches no node or market.

Steps:

1. Follow the browser QA walk in [QA_WORKFLOWS.md](../../../QA_WORKFLOWS.md) §J (the last two items): tick the mainnet selling setting, win a name in the mock's auction engine, Sell it, and move blocks with `__webqa_mine(n)` in the console.

Expected:

- As written there: an unpublished Listed listing offers `Publish on LearnHNS Market` and, once published, no Unpublish; the listing walks every state with only the actions the backend fixture allows; the name's owner actions say "this name is locked for sale through Shakedex" until the listing is Cancelled.
- The market status texts per status are not shown by the mock (it never reports one); they are pinned by `my-listings.test.tsx`.
- Passes if the walk matches; it says nothing about a real node.

- **Blocks mined:** 0
- **Evidence:** browser screenshots of the Publish button and the states.
- **Cleanup / end state:** stop the dev server.

## C5 — Not GUI-checkable on regtest

- **Priority:** P3
- **Was:** T7 fixes recheck C5 (with M4 and the mainnet half of M1 there)
- **Verdict:** always BLOCKED ✗ on regtest; record it with the reason.

| What | Spec | Why not on regtest | Covered instead by |
|---|---|---|---|
| The LearnHNS Market list: the hidden-listings toggle, `Couldn't check` rows with Retry, Load more, a market row's kind from its step times, the SPV and Explorer "Listings can't be checked yet" state | R5, R8, R12 | no market off mainnet; SPV or Explorer would mean re-pointing a shared profile | vitest `market-page.test.tsx`, `market-model.test.ts`; `market_row_kind_follows_the_step_times`; `list_market_reads_the_profile_it_is_given` |
| A link import fetching `proof.json`; a published market fee pre-ticked | R5, R11 | needs LearnHNS (`fee_info`) | vitest `market-page.test.tsx :: imports a market link as a link`, `listing-detail.test.tsx :: prices a LearnHNS link as from the market` |
| Publishing, the pending listing, uploads, `expiresAt` refresh, keeping a listing on the market, market statuses, the cancel and sale reports | R23, R24 (re-upload), R25, R28 (market report) | LearnHNS refuses every write off mainnet | vitest `my-listings.test.tsx` (every market status text) |
| The mainnet experimental flag refusing a purchase or a listing; the "Buying on mainnet is off." banner and `Turn on buying` | R15, R29 | needs a mainnet profile | `mainnet_purchase_needs_experimental_flag`; the vitest gate tests (`market-page.test.tsx :: offers to turn buying on from the banner, and the panel's button does it too`); G3 checks the flag does nothing on regtest |
| A name expiring before a purchase could be finalized | R9 | needs a name within ~300 blocks of expiry (~4 700 blocks after its last FINALIZE) | `shakedex_listing_expiring_before_finalize_is_not_offered`; every buyable row in B shows no `Expires soon` tag |
| The Lock dialog's near-expiry warning and refusal | R31 | needs a name renewed ≥ 3 200 blocks ago (refusal ≥ 4 980) | `shakedex_lock_refused_near_expiry`; S2 checks a fresh name shows no warning |
| A listing expiring with its name | R19 (Expired) | 5 000 blocks | `shakedex_listed_lock_expires_with_its_name`; R6 observes an existing expired row |
| Reorgs, rebroadcast after missing blocks, mempool expiry, "Lost, paid" by expiry, a purchase beating a broadcast cancel | R13, R22, R28 | needs `invalidateblock`/`setmocktime`, forbidden on the shared node | `shakedex_purchase_follows_reorgs_of_its_own_blocks`, `shakedex_purchase_the_node_lost_is_given_up_after_mempool_expiry`, `shakedex_name_expired_before_finalize_is_lost_with_the_price_paid`, `shakedex_cli_purchase_beats_our_cancel`, `shakedex_namehold_listing_sold_to_the_cli_is_sold` |
| Byte parity of scripts, steps, purchases, FINALIZEs and cancels; fees on vsize; the lock self-check | R1, R4, R18 | not observable in a GUI | `shakedex_vector_tests`; the `*_pays_its_fee_rate_on_vsize` live tests; unit tests |
| Ledger profiles | R16 | no device | vitest gate tests |
