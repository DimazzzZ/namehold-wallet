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

## M5 — The mainnet Market page in the browser mock

- **Priority:** P2
- **Covers:** R5, R8, R12, R14, R15 (the page's layout and texts on mainnet)
- **Preconditions:** mode C ([environment](../../environment.md#modes)): `pnpm dev`, whose mock profile is mainnet with LearnHNS listings, unavailable listings and two purchases; a fresh page load (the mock keeps the buying setting until reload). Window 1440×800.

Steps:

1. Open `/market`; read the page top to bottom without scrolling.
2. Click `▸ 5 unavailable listings hidden…`; read; click it again.
3. Select `.studio`; read the panel. Scroll the page down until the list's foot is in view; read the panel's button.
4. Press `Load more`; read the list's foot.
5. Press the banner's `Turn on buying`; read.

Expected:

- The banner "Buying on mainnet is off. Shakedex swaps on mainnet are still experimental. …" with a filled `Turn on buying`; underlined tabs `Browse` / `My listings`; `Import listing…` outlined (white).
- `Your purchases`: `.pixelart` "Paid 410.2 HNS", a green bar, "Ready to finalize" and an outlined `Finalize` (the banner holds the page's one filled button); `.harbor` "Paid 320 HNS", "146 of 288 blocks" and `~1 d 0 h left`.
- The list: `.nightowl` tagged `Expires soon`, prices such as `1,200 HNS` with `↓ 1,100 HNS in ~6 h` for `.atelier` and `Fixed price` for fixed rows, `✓ Verified`; `.oldmarket` with `—`, `Couldn't check` and `Retry` beside it, inside the card. The first verified row is selected.
- The panel of a selected listing: above its full-width outlined `Turn on buying`, the amber line "Buying on mainnet is off, so this button turns it on first. Nothing is bought until you press Buy."; its steps read `Buy`, `Wait ~2 days` ("288 blocks while the swap settles. Track it in Your purchases.") and `Finalize`, with no `How it works` heading.
- Step 2: the toggle reads `▸ 5 unavailable listings hidden: 3 sold, 1 failed the check, 1 not valid yet`, then `▾ Hide 5 unavailable listings` with the rows on a gray background, not selectable.
- Step 3: `.studio`'s panel shows "Price drops over time · LearnHNS Market", its price steps and "Waiting gets a lower price, but anyone can buy it first."; at the top of the page and scrolled, the panel's button stays on screen, its lower sections scrolling inside the panel.
- Step 4: `.marigold` and `.harborlight` join the list, and the foot reads "All LearnHNS listings loaded".
- Step 5: the toast "Mainnet buying turned on · also in Settings › Shakedex"; the banner is gone and the panel's button becomes the filled `Buy .<name> for <total> HNS`; `Finalize` in Your purchases stays outlined beside it.
- Passes if the page matches; it says nothing about a real node or market.

- **Blocks mined:** 0
- **Evidence:** browser screenshots `M5-1-top`, `M5-2-hidden-open`, `M5-3-studio-scrolled`, `M5-4-all-loaded`, `M5-5-buying-on`.
- **Cleanup / end state:** reload the page (the mock forgets the setting); stop the dev server.

## M6 — When buyers stop seeing a listing, in the browser mock

- **Priority:** P2
- **Covers:** R35
- **Preconditions:** mode C ([environment](../../environment.md#modes)): `pnpm dev` on a fresh page load. The mock's mainnet chain stands at height 100 000 and knows the expiry of two names: `urgent` (1 000 blocks from its end) and `example` (10 000 blocks); its signer is unlocked.

Steps:

1. Open `/market` → `My listings` → `Restore a lock by name` → type `urgent` → `Restore`; read the new row.
2. Restore `dexsale` the same way; read its row.
3. In the browser console run `__webqa_mine(1000)`; switch to `Browse` and back to `My listings`; read both rows.

Expected:

- Step 1: the toast "Restored .urgent: Locked — listing details unknown"; the row reads `Locked — listing details unknown` with `Cancel listing` and `Import its listing file`, and one amber line: "Buyers stop seeing this listing in ~6 days, when the name gets too close to expiring to finalize a purchase. Cancel it and bring the name home to renew it, then list it again." (979 blocks at ten minutes a block, whole days counted down).
- Step 2: the `dexsale` row has no such line (the mock knows no expiry for it).
- Step 3: the `urgent` row reads "Buyers no longer see this listing: the name is too close to expiring to finalize a purchase. Cancel it and bring the name home to renew it."; `dexsale` still has no line.
- Passes if the line counts days on mainnet, turns to "no longer" past the point, and is absent where the count is unknown. The regtest wording (`in N blocks`) is not reachable here or on the shared regtest chain ([C5](#c5--not-gui-checkable-on-regtest)).

- **Blocks mined:** 0
- **Evidence:** browser screenshots `M6-1-urgent-days`, `M6-3-no-longer`.
- **Cleanup / end state:** reload the page (the mock forgets the restored locks and its height); stop the dev server.

## C5 — Not GUI-checkable on regtest

- **Priority:** P3
- **Was:** T7 fixes recheck C5 (with M4 and the mainnet half of M1 there)
- **Verdict:** always BLOCKED ✗ on regtest; record it with the reason.

| What | Spec | Why not on regtest | Covered instead by |
|---|---|---|---|
| The LearnHNS Market list: the hidden-listings toggle, `Couldn't check` rows with Retry, Load more, a market row's kind from its step times, the SPV and Explorer "Listings can't be checked yet" state | R5, R8, R12 | no market off mainnet; SPV or Explorer would mean re-pointing a shared profile | vitest `market-page.test.tsx`, `market-model.test.ts`; `market_row_kind_follows_the_step_times`; `list_market_reads_the_profile_it_is_given`; the layout and texts in the browser mock: M5 |
| A link import fetching `proof.json`; a published market fee pre-ticked | R5, R11 | needs LearnHNS (`fee_info`) | vitest `market-page.test.tsx :: imports a market link as a link`, `listing-detail.test.tsx :: prices a LearnHNS link as from the market` |
| Publishing, the pending listing, uploads, `expiresAt` refresh, keeping a listing on the market, market statuses, the cancel and sale reports | R23, R24 (re-upload), R25, R28 (market report) | LearnHNS refuses every write off mainnet | vitest `my-listings.test.tsx` (every market status text) |
| The mainnet experimental flag refusing a purchase or a listing; the "Buying on mainnet is off." banner and `Turn on buying` | R15, R29 | needs a mainnet profile | `mainnet_purchase_needs_experimental_flag`; the vitest gate tests (`market-page.test.tsx :: offers to turn buying on from the banner, and the panel's button does it too`); G3 checks the flag does nothing on regtest |
| A name expiring before a purchase could be finalized | R9 | needs a name within ~300 blocks of expiry (~4 700 blocks after its last FINALIZE) | `shakedex_listing_expiring_before_finalize_is_not_offered`; every buyable row in B shows no `Expires soon` tag |
| The Lock dialog's near-expiry warning and refusal | R31 | needs a name renewed ≥ 3 200 blocks ago (refusal ≥ 4 980) | `shakedex_lock_refused_near_expiry`; S2 checks a fresh name shows no warning |
| A listing expiring with its name | R19 (Expired) | 5 000 blocks | `shakedex_listed_lock_expires_with_its_name`; R6 observes an existing expired row |
| Reorgs, rebroadcast after missing blocks, mempool expiry, "Lost, paid" by expiry, a purchase beating a broadcast cancel | R13, R22, R28 | needs `invalidateblock`/`setmocktime`, forbidden on the shared node | `shakedex_purchase_follows_reorgs_of_its_own_blocks`, `shakedex_purchase_the_node_lost_is_given_up_after_mempool_expiry`, `shakedex_name_expired_before_finalize_is_lost_with_the_price_paid`, `shakedex_cli_purchase_beats_our_cancel`, `shakedex_namehold_listing_sold_to_the_cli_is_sold` |
| Byte parity of scripts, steps, purchases, FINALIZEs and cancels; fees on vsize; the lock self-check | R1, R4, R18 | not observable in a GUI | `shakedex_vector_tests`; the `*_pays_its_fee_rate_on_vsize` live tests; unit tests |
| Ledger profiles | R16 | no device | vitest gate tests |
| My listings' line saying when buyers stop seeing a Listed or Restored listing (`in N blocks` on regtest) | R35 | needs a listed name within ~300 blocks of the point buyers stop seeing it, about 4 700 blocks after the FINALIZE into the lock (which renews the name) | `shakedex_verify_tests::{blocks_until_unbuyable_counts_down_to_r9s_margin, buyable_blocks_by_name_reads_each_names_expiry_once, unbuyable_warn_window_is_at_most_half_the_renewal_window}`, `shakedex_my_listings_tests::a_listed_row_says_how_long_buyers_still_see_it`, vitest `my-listings.test.tsx :: R35: when buyers stop seeing the listing`; the mainnet wording in the browser mock: M6; every Listed row in the S and R cases shows no such line |
| A listing managed from another device: the mark, the amber `Managed from another device: …` status line with the other device's price, prices `— on this device`, `Manage from this device` and its dialog, `Import its listing file` on a Listed row taking it over, Lower price refused, the Cancel dialog's sentence on the other device's price, the export line; no upload over the other device's copy; the take-over's refusals | R36–R42, R44, R45 | the mark comes only from a market job reading LearnHNS Market's copy of the listing, and the market jobs return before any read off mainnet (R23); the browser mock has one device. Importing the other device's file on regtest takes nothing over: a Listed row offers `Import its listing file` only while marked (a Restored lock's import is R1) | live `shakedex_two_devices_of_one_phrase_take_a_listing_over`; `shakedex_jobs_tests::{foreign_step_on_the_market_marks_the_listing_and_nothing_is_uploaded, copies_without_a_foreign_step_are_never_flagged, first_upload_does_not_go_over_a_foreign_step, advance_does_not_go_over_a_foreign_step, a_listing_managed_elsewhere_gets_no_upload_and_keeps_its_mark_through_a_reset, a_listing_managed_elsewhere_still_reports_its_sale_and_its_cancel, daemon_and_app_upload_nothing_for_a_listing_managed_elsewhere, later_expiry_on_the_market_is_kept_and_an_earlier_one_replaced}`, `shakedex_listing_state_tests::{managed_elsewhere_mark_lands_only_on_the_listing_read, a_listing_back_before_its_lock_loses_its_mark, a_listing_managed_elsewhere_is_still_sold_from_the_chain}`, `shakedex_sell_tests::{lower_refused_while_managed_elsewhere, cancel_prompt_of_a_listing_managed_elsewhere_names_the_other_devices_price, manage_here_adopts_the_other_devices_step_and_clears_the_mark, manage_here_refuses_and_writes_nothing, take_over_write_takes_only_the_listing_as_read, other_devices_file_takes_a_listing_managed_elsewhere_over, other_devices_file_that_is_not_this_listings_is_refused}`, vitest `my-listings.test.tsx`; the chain-fact wording of R43 is checked on regtest by R9 |
| Speed up pulling a stuck parent into a full block; a child dropped with "The transaction it sped up (<txid>) did not confirm, so this one can never confirm. Nothing was paid by either." when its parent leaves the mempool; the 6-hour expiry and relay-floor refusals | Speed up R5, R6, R15, R17 | the shared node never fills a block, and emptying its mempool means a restart or 72 hours; the wallet never builds a parent below the floor | live `live_cpfp_parent_and_child_confirm_together`, `live_cpfp_child_of_a_vanished_parent_says_so` (on the capped node of `scripts/regtest.sh start-cpfp`); `speed_up_tests::{parent_close_to_its_mempool_expiry_is_refused, parent_below_the_relay_floor_is_refused, a_childs_death_names_its_parent}` |
| Speed up on a Ledger profile | Speed up R13 | no device; no child has been signed on a device yet | `speed_up_cmd_tests::the_ledger_guard_passes_the_childs_plan` |
