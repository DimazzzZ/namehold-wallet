# Multiple bids per name

## 1. Summary

The wallet lets you bid on one name as many times as you like. Everything
downstream of the bid had been written when that was impossible: reveal,
redeem, the capability gates, the guided panel, the bids panel and the chain
scanner all assumed a name had at most one bid of ours, and a name had at most
one auction ever. Each assumption was a way to lose money quietly — a second
bid that never got revealed, a lockup that could not be reclaimed, a reveal
value attached to the wrong bid, a deadline warning that went silent on the
bids still at risk.

This spec makes the **auction**, not the name, the unit everything is keyed
by, and makes every multi-bid operation cover every bid. Revealing a name
reveals all of its bids in one transaction; redeeming reclaims every losing
reveal in one transaction, including when you also won. A bid from an auction
that has lapsed is counted as lapsed and named as a stranded lockup rather
than silently mixed into the live one. And because most of these steps only
resolve when a block arrives, each one says so while it waits, instead of
showing a stale state the user reads as a bug.

## 2. Terms

- **Auction** — one OPEN of a name and the bidding/reveal windows that follow
  it. Identified by its **start height**: the block the OPEN was mined in,
  which hsd writes into item `[1]` of every BID and REVEAL covenant for that
  auction (`u32le_hex` in `chain_scan.rs` decodes it) and reports as
  `info.height` from `getnameinfo`.
- **Lapsed auction** — an auction whose reveal window closed. The name becomes
  available and can be opened again; a new OPEN starts a **new** auction with
  a new start height.
- **Commitment** — a row of `bid_commitments`: our plaintext bid value, lockup
  and nonce for one bid, the only place the blind can be reversed from. Keyed
  by `blind_hex`, and since migration 030 it records `name_start_height`.
- **Indexed bid** — a row of `name_bid_outpoints`: a BID covenant the chain
  scanner saw, ours or anyone's. Keyed by `(network, bid_txid, bid_vout)`, and
  since migrations 028/029 carries `network` and `name_start_height`.
- **Current auction** — the auction of the name state the UI is looking at.
  Commitments and indexed bids for any other start height belong to a
  different auction and are not part of it.
- **Stranded lockup** — a BID coin of ours still unspent, from an auction that
  is not the current one. Its only valid spend was a REVEAL inside that
  auction's window, so it is unrecoverable; it is reported, never offered as
  an action.
- **Registered** — the name's owner coin is at `COV_REGISTER` or later.
  Distinct from **owned**: during REVEAL hsd reports the highest revealer as
  the owner, so a wallet leading its own auction is "owned" while holding
  nothing but a REVEAL coin. `nameIsRegistered` carries the distinction to the
  UI.
- **Live / upcoming / absent** — the three states a modal section can be in
  (R19). Live has at least one allowed action. Upcoming belongs to a later
  stage and renders as one muted line. Absent cannot apply and renders
  nothing.
- **Waiting for a block** — we broadcast a transaction and the chain has not
  mined it. Not an error and not a phase: a state the UI names so the user
  does not read the unchanged phase as a failure.

## 3. Requirements

### Scoping: one auction at a time

**R1 — The bid index is per network.** The scanner cursor and every indexed
bid carry the `network` of the profile that recorded them, and a read never
crosses networks: a mainnet height in the cursor must not read as coverage of
a regtest tip. The scanner runs only for a network it recognises.
*Enforced:* `commands/chain_scan.rs` (`scan_network`, all `WHERE network = ?`),
migration `028_chain_scan_network_scope.sql`.
*Pinned:* `read_cmd_tests::read_name_bids_ignores_another_networks_cursor_and_index`.

**R2 — The bids panel shows this auction's bids and no others.** Indexed bids
are filtered by the current auction's start height. A name with no auction
open shows nothing, not the bids of the auction that lapsed.
*Enforced:* `commands/read.rs::read_name_bids`, migration
`029_bid_auction_scope.sql`.
*Pinned:* `read_cmd_tests::read_name_bids_shows_only_the_current_auctions_bids`,
`read_cmd_tests::read_name_bids_shows_nothing_when_the_name_has_no_open_auction`,
`live_node_it::live_reopened_name_scopes_its_bids_and_strands_the_old_lockup`.

**R3 — "N of yours" counts this auction's commitments.** Commitments are
filtered by `name_start_height` before anything is derived from them, so the
modal header cannot disagree with the panel below it. A commitment recovered
from the chain has no known auction and still counts — showing a bid that may
be dead is a wrong number; hiding a live one loses the lockup.
*Enforced:* `commands/names.rs::find_name_action_context(.., auction_start)`,
migration `030_bid_commitment_auction.sql` (backfilled per network from the
stored reveal deadline).
*Pinned:* `names_action_context_tests::find_name_action_context_counts_only_this_auctions_bids`,
`names_action_context_tests::find_name_action_context_keeps_a_commitment_with_no_known_auction`.

**R4 — A lapsed auction leaves no state behind.** When the node reports a name
with a null `info`, every column derived from `info` is cleared, not just
`state` — otherwise `height` still names the dead auction and the row
contradicts its own `raw_json`.
*Enforced:* `noncustodial/sync.rs::upsert_name_state` (null-info branch).
*Pinned:* `sync::tests::upsert_name_state_clears_info_derived_columns_when_the_auction_is_gone`.

### Covering every bid

**R5 — Revealing a name reveals every bid of ours in the current auction.**
One transaction, one REVEAL output per bid with an unspent BID coin. Bid coins
are found by name hash across the wallet, not at one commitment's address —
each bid lands on its own rotated address.
*Enforced:* `commands/names.rs::build_reveal_draft` / `build_reveal_draft_inner`.
*Pinned:* `live_node_it::live_reveal_covers_every_bid_this_wallet_placed_on_the_name`,
`live_node_it::live_multi_bid_lifecycle_leaves_no_coin_stranded`.

**R6 — The reveal txid is stamped on the bid that was revealed, and only it.**
`set_bid_reveal_txid` is keyed by `blind_hex`. Stamping by name marked every
commitment revealed, which silenced the reveal-deadline warning for exactly
the bids still at risk. The batch path stamps too.
*Enforced:* `db/queries.rs::set_bid_reveal_txid`, called from both reveal
builders in `commands/names.rs` (`build_reveal_draft_inner` and the batch
reveal command) — the plan in `noncustodial/actions.rs` carries no txid.
*Pinned:* `names_cmd_tests::build_reveal_draft_persists_reveal_txid_on_its_commitment`,
`deadlines_cmd_tests::revealed_bid_is_excluded_even_if_the_window_would_be_imminent`.

**R7 — Redeeming reclaims every losing reveal in one transaction.** All
unspent REVEAL coins for the name except the owner's are inputs. Owning the
name does not disqualify a redeem: winning with one of your bids and losing
with the others is the ordinary outcome of bidding twice. Holding no reveal
besides the winner does disqualify it.
*Enforced:* `commands/names.rs::build_redeem_draft` / `build_redeem_draft_inner`,
`can_redeem = phase == "CLOSED" && redeemable_reveal_count > 0`.
*Pinned:* `name_capabilities_tests::cap_closed_phase_can_redeem_own_losing_bids_while_owning_the_name`,
`name_capabilities_tests::cap_closed_phase_cannot_redeem_when_the_only_reveal_won`,
`live_node_it::live_redeem_when_won_rejected`.

**R8 — A registered name with losing reveals still asks to be redeemed.** The
guided panel reports `lostNeedsRedeem` for a CLOSED, owned, registered name
that still holds a reveal coin, rather than "no urgent action".
*Enforced:* `commands/names.rs::derive_auction_task_state`.
*Pinned:* `auction_capabilities_tests::closed_with_reveal_coin_yields_lost_needs_redeem`,
`name_capabilities_tests::task_state_closed_lost_has_reveal_coin`.

**R9 — Each reveal reports the value its own reveal disclosed.** The scanner
pairs a REVEAL with the BID coin spent at the same input index — which is how
`rules.verifyCovenants` pairs them — not with "the earliest bid not yet
matched", a heuristic indistinguishable from the truth only while a wallet
holds one bid per name.
*Enforced:* `commands/chain_scan.rs` (`vin[i]` → `WHERE bid_txid = ? AND bid_vout = ?`),
migration `031_rescan_reveal_pairing.sql` re-scans existing wallets, because
values recorded under the old rule may sit on the wrong bid and the scanner
never revisits a block it has passed.
*Pinned:* `chain_scan_tests::scan_block_pairs_each_reveal_with_the_bid_it_spends`,
`live_node_it::live_scanner_pairs_each_reveal_with_its_own_bid`.

**R10 — A stranded lockup is named, with its amount.** The capabilities carry
`strandedBidCount` and `strandedLockupDoos`, and the guided panel states them.
*Enforced:* `commands/names.rs::find_name_action_context`,
`src/components/name-actions/` (acquisition panel).
*Pinned:* `names_action_context_tests::find_name_action_context_reports_a_stranded_bid_from_a_lapsed_auction`,
`names_action_context_tests::find_name_action_context_does_not_strand_a_spent_bid`,
`name-acquisition.test.tsx :: names the lockup stranded in an earlier auction of the same name`.

### Gates

**R11 — Owning a name is not yet the right to spend it.** Update, Transfer,
Cancel transfer, Renew and Revoke require `owns_name` **and** an owner coin at
`COV_REGISTER` or later. During REVEAL, `getnameinfo` already names the
highest revealer as owner, and a REVEAL coin may become only a REGISTER or a
REDEEM — every one of those transactions would have been refused by the node.
The reason says which half is missing (`"the name is not registered yet"`).
`can_register` is untouched: it is the action that moves the coin to REGISTER.

Signing a message for a name is the same claim without a transaction, and is
refused under the same rule. `get_name_coin` resolves whatever
`tracked_name_states.owner_txid` points at and filters on no covenant, so
during REVEAL it hands back our own REVEAL coin; the command signed it and
returned a well-formed proof of ownership that every verifier resolves as
false.
*Enforced:* `commands/names.rs::build_name_action_capabilities`,
`commands/tx.rs::sign_name_message`.
*Pinned:* `names::tests::ownership_actions_need_a_registered_name_not_just_ownership`,
`names::tests::ownership_actions_stay_available_once_registered`,
`sign_name_message_tests::rejects_a_name_whose_owner_coin_is_still_a_reveal`.

**R11b — Every capability answers for a transfer in flight.** hsd lets a
TRANSFER coin go to UPDATE, RENEW, FINALIZE or REVOKE, and consensus decides
which of those a button may offer.

- **Update** is refused: its UPDATE branch *is* the cancel, so a button
  labelled "edit your DNS records" was a way to lose a transfer in flight.
- **Renew** is refused for the same reason. hsd's RENEW handler runs
  `ns.setTransfer(0)` exactly as UPDATE does, so "extend my registration"
  ended a transfer and said nothing about transfers.
- **Transfer** is refused: a TRANSFER coin may become an UPDATE, RENEW,
  FINALIZE or REVOKE and nothing else, so a second one is a transaction the
  node rejects.
- **Revoke** stays offered. Consensus allows it from a TRANSFER coin, and
  destroying the name is exactly what that button says it does.
- **Cancel transfer** requires a transfer to cancel — the condition
  `can_finalize` has always carried — instead of building a no-op UPDATE that
  costs a fee.

*Enforced:* `commands/names.rs::build_name_action_capabilities`.
*Pinned:* `names::tests::update_is_refused_while_a_transfer_is_pending`,
`names::tests::cancel_transfer_is_refused_when_no_transfer_is_pending`,
`names::tests::renew_is_refused_while_a_transfer_is_pending`,
`names::tests::transfer_is_refused_while_a_transfer_is_already_pending`.

**R11c — Finalize waits out the transfer lockup.** hsd refuses a FINALIZE
until `transfer + transferLockup` blocks have passed (`bad-finalize-maturity`),
so offering it the moment a transfer is mined sends the user at a transaction
the node throws away — two days on mainnet, ten blocks on regtest. The gate
compares the transfer's height against the tip and the reason counts the
blocks left rather than only saying no. It prefers the live tip, fetched once
per call, and falls back to the persisted estimate when no synced node answers;
with either height unknown the action stays offered, because refusing one the
node would accept is its own kind of wrong.
*Enforced:* `commands/names.rs::build_name_action_capabilities`,
`commands/names.rs::evaluate_name_action_capabilities` (live tip).
*Pinned:* `names::tests::finalize_waits_out_the_transfer_lockup`,
`names::tests::finalize_is_not_blocked_when_the_lockup_is_unknown`,
`names::tests::a_live_tip_replaces_the_persisted_estimate`.

**R11d — A batched owner spend is still an owner spend.** The draft a batch
builder persists records itself as `batch-<action>`, so an owner-spend check
matching only the singular action names read a batched transfer as no transfer
at all — collapsing ownership the moment one went out. The match strips a
`batch-` prefix, which leaves `batch-bid` and `batch-redeem` correctly saying
nothing about ownership.
*Enforced:* `commands/names.rs::find_name_action_context`.
*Pinned:* `names_action_context_tests::find_name_action_context_recognises_a_batched_owner_spend`.

**R12 — A name whose auction lapsed can be opened again.** Only an
*unconfirmed* OPEN coin counts as a pending OPEN. The OPEN output is a
zero-value marker nothing ever spends, so treating "we hold one" as "one is
pending" made every name this wallet had ever opened permanently un-openable.
*Enforced:* `commands/names.rs::find_name_action_context` (its
`has_pending_open_coin` value) →
`db/queries.rs::has_unconfirmed_covenant_utxo_by_name_hash`.
*Pinned:* `names_action_context_tests` (pending-open cases),
`build_open_draft_tests`, `live_node_it::live_double_open_and_double_bid_guarded`.

### Saying what is happening

**R13 — An action sent but not yet mined says so, everywhere it appears.** The
name modal, the auctions list, the guided panel and the Owned Names table's
State column all read the same `pendingBroadcastAction` and render "waiting for
a block" naming the action, rather than the unchanged phase. The State column
was the last surface that did not, so a row read "Owned" while the modal it
opened read "Redeem · waiting for a block".
*Enforced:* `db/queries.rs::pending_broadcast_action_for_name`,
`src/lib/auction.ts` (`pendingBroadcastText`, `pendingBroadcastBadge`),
`src/components/WalletView.tsx` (State column).
*Pinned:* `auction.test.ts :: names the action that is waiting for a block`,
`wallet-view.test.tsx :: defers to what is in flight, like the modal it opens`,
`auction-positions.test.tsx :: a broadcasted open the node/explorer hasn't caught up to (waitingForBidding) shows Waiting for Bidding / View`,
`name-acquisition.test.tsx :: says the OPEN is waiting to be mined, with no Open button`.

**R13b — A pending transfer is a task, not a phase that never arrives.** hsd
has six name states — OPENING, LOCKED, BIDDING, REVEAL, CLOSED, REVOKED — and
TRANSFER is not among them: a transfer leaves the state at CLOSED and shows
itself through `info.transfer`. A task derivation keyed on a `"TRANSFER"` phase
string therefore never fired, and every name being transferred fell through to
"no urgent action" — on a name whose one remaining action is to finalize it.
The task reads the transfer the node actually reports, and ranks behind the
renewal alarm but ahead of everything quiet: losing the name outranks
completing a transfer of it, and nothing else does. That includes R8: a name
mid-transfer that still holds losing reveals reports the transfer, not the
redeem, because the transfer is the task the user started and the name is on
its way out of the wallet, while the reveals are reclaimable at any time —
from the bids panel, the manual auction actions, or Redeem Selected.
*Enforced:* `commands/names.rs::derive_auction_task_state`.
*Pinned:* `auction_capabilities_tests::a_recorded_transfer_yields_transfer_pending_finalize`.

**R13c — A reclaimed lockup is money again.** REDEEM is classified as
spendable, not name-bound. hsd draws the same line in
`Covenant::isNonspendable()`, which returns false for NONE, OPEN and REDEEM: a
redeemed coin spends like any other output. Without this, HNS reclaimed from
losing bids landed in `name_control`, so the balance card did not show it as
spendable and coin selection would not draw on it — the user had just paid a
fee to get it back and it stayed invisible. OPEN stays name-bound on purpose:
hsd would spend it too, but it is a zero-value marker, so counting it as liquid
would add an input and no value. No migration is needed; the sync upsert
rewrites `spend_class` on conflict.
*Enforced:* `noncustodial/sync.rs` (spend-class match).
*Pinned:* `sync::tests::redeemed_value_counts_as_liquid_balance`.

**R13d — A confirm dialog counts every output the action carries.** A name
action can carry several: revealing a name bid on more than once emits one
REVEAL per bid, and redeeming reclaims one per losing reveal. Reporting
`outputs[0]` offered a live wallet a redeem of three reveals worth 28 HNS with
12 on the dialog — the one figure a user checks before signing, wrong on every
multi-bid action. The total sums every output except change, which the plan
already identifies by index.
*Enforced:* `commands/names.rs` (`send_total_doos` in the draft summary).
*Pinned:* `build_redeem_draft_tests::build_redeem_draft_totals_every_output_it_reclaims`.

**R14 — Our own unmined bid appears in the bids panel, counted apart.** A bid
we broadcast but the chain has not indexed is appended to the list as
`pending: true`, so placing a second bid does not look like it vanished. Only
commitments with a `bid_txid` qualify.
*Enforced:* `commands/read.rs::merge_indexed_bids`.
*Pinned:* `read_cmd_tests::merge_indexed_bids_appends_our_own_unmined_bid_as_pending`,
`read_cmd_tests::merge_indexed_bids_commitment_without_txid_never_matches`,
`name-bids-panel.test.tsx :: lists a bid of ours still waiting for a block, and counts it apart`,
`live_node_it::live_own_bid_is_pending_before_the_block_and_indexed_after`.

**R15 — Activity shows the name for every name action.** OPEN, BID and
FINALIZE carry the name in their covenant and are decoded; the covenants that
carry no name (REVEAL, REDEEM, REGISTER, UPDATE, RENEW) fall back to the name
in this wallet's own draft.
*Enforced:* `commands/history.rs::classify_tx`, `src/lib/activity.ts`.
*Pinned:* `history_cmd_tests::history_happy_path_bid_decodes_name`,
`activity.test.ts :: draft fallback: backend nameValueDoos null → uses matched draft send total`.

**R16 — The stated auction window is this network's.** Bidding and reveal
periods come from the profile's `NameParams`; no copy claims a duration. A
one-block period is singularised.
*Enforced:* `src/lib/auction.ts::auctionWindowText`, `AUCTION_PHASE_GUIDE`,
capabilities fields `auctionBiddingBlocks` / `auctionRevealBlocks`.
*Pinned:* `auction.test.ts :: reports the network's real periods`,
`auction.test.ts :: does not hardcode an auction duration`.

**R17 — A disabled action says why.** Reasons render through the `Tooltip`
component, never a native `title`: a disabled button has
`pointer-events: none`, so a native tooltip can never be shown. A repo lint
enforces the rule.
*Enforced:* `src/components/ui/Tooltip.tsx`,
`src/components/name-actions/ActionHint.tsx`, `ActionReasonBanner.tsx`,
`scripts/lint-native-title.mjs` (`pnpm lint:native-title`).
*Pinned:* `tooltip.test.tsx :: works on a disabled control, where a native title never would`,
`action-hint.test.tsx`, `action-reason-banner.test.tsx`.

**R18 — A table row is not a click target; its controls are.** An Owned Names
row has no click handler — the name, the two block heights and Manage each act
for themselves, so a click on one of them opens one dialog, not two. The
shared `DataTable` still offers an opt-in row click and ignores clicks that
came from a control handling them itself.
*Enforced:* `src/components/WalletView.tsx`, `src/lib/rowClick.ts`,
`src/components/ui/DataTable.tsx`.
*Pinned:* `wallet-view.test.tsx :: clicking the row itself does nothing — only its controls act`,
`wallet-view.test.tsx :: clicking a cell button in an Owned Names row opens only that button's dialog`,
`rowClick.test.ts`.

**R19 — The modal's sections are a map of the lifecycle, not a catalogue of
verbs.** Each of the three — manual auction actions, DNS records, ownership
(which now contains signing) — is **live** when something inside is allowed,
**upcoming** when it belongs to a later stage (one muted line naming what
unlocks it, no controls), or **absent** when it cannot apply. Four rules carry
the weight: Register lives in the records section, so that section opens
before the name is registered; a pending transfer closes it again (R11b),
read from the backend's `transferPending` rather than re-derived from the
phase; while a broadcast waits for a block every section is absent, since
offering alternatives then only invites a competing transaction; and the
advanced area — toggle and container both — appears only when at least one
section is live, so an upcoming line is shown beside a live section and never
as a menu whose whole content is "come back later". Every gate that read
`ownsName` — the section filter, auto-expand, the toggle and its label, the
read-only DNS suppression, and the "Owned by this wallet" badge — now reads
the stage.
*Enforced:* `src/lib/nameSections.ts::resolveSections`,
`src/components/NameActionsModal.tsx`,
`src/components/name-actions/UpcomingSection.tsx`,
capability fields `nameIsRegistered` and `transferPending`.
*Pinned:* `nameSections.test.ts` (the stage matrix),
`name-modal-sections.test.tsx`,
`name-actions-bid-gate.test.tsx :: offers no advanced section at all during OPENING, so no bid can be invited`,
`name-actions-gating.test.tsx :: states the reason once and offers no menu when the node can't write`.

## 4. Explicitly not enforced

- **A stranded lockup is not recoverable.** R10 reports it; nothing reclaims
  it. A BID output's only valid spend is its own REVEAL inside that auction's
  reveal window (`rules.verifyCovenants`: "Bid has to go to a reveal"), so
  once the window closes the coin is dead. The wallet says so rather than
  offering an action that cannot exist.
- **Nothing stops you from outbidding yourself.** Placing several bids on one
  name is the supported case, not a mistake to be guarded; the wallet warns
  about the lockup, it does not refuse.
- **The wallet does not bid, reveal or redeem on its own.** Every one of these
  is a transaction the user signs. "Waiting for a block" is a statement about
  the chain, not a background retry.
- **A commitment with no recorded auction is not attributed to one.** R3
  counts it in the current auction deliberately. It is not proof the bid is
  live.
- **An upcoming section is not shown on its own.** R19 renders it as one muted
  line naming what unlocks it, but only inside the advanced area, which needs
  a live section to exist at all. On a name where nothing is actionable — a
  reveal already sent, say — there is no menu and no line: a menu whose whole
  content is "come back later" is the empty menu R19 exists to remove. It is
  also not expandable, since expanding would reveal nothing.
- **The three states are not permissions.** A live section can still hold
  buttons that are individually refused — an owner coin that has not synced,
  a locked signer — each with its own reason. The section answers "does this
  stage have this?", the capability answers "can you press it?".

## 5. Known gaps

- **`DataTable` still carries an `onRowClick` prop with no caller.** Kept
  because the guard in R18 is the thing worth keeping, and the next table that
  wants a row click should get the guarded version.
- **Commitments are ordered by `created_at`, which has second resolution.**
  Two bids placed in the same second order arbitrarily. No longer load-bearing
  — every path that used to resolve "the newest commitment" now resolves all
  of them by name hash — but the column is still imprecise.
- **Migration 031 re-scans from height 0 on upgrade.** The only correct fix
  for values recorded under the old pairing rule, but a full re-scan on a
  mainnet node is slow.

## 6. Pointers

**Backend**
- `src-tauri/src/commands/chain_scan.rs` — scanner: network scope, block
  shape, auction start decoding, reveal→bid pairing.
- `src-tauri/src/commands/names.rs` — action context, capabilities, task
  state, reveal/redeem draft builders.
- `src-tauri/src/commands/read.rs` — `read_name_bids`, `merge_indexed_bids`.
- `src-tauri/src/commands/history.rs` — `classify_tx` name decoding.
- `src-tauri/src/db/queries.rs` — `set_auction_heights`,
  `set_bid_reveal_txid`, `has_unconfirmed_covenant_utxo_by_name_hash`,
  `pending_broadcast_action_for_name`.
- `src-tauri/src/noncustodial/sync.rs` — `upsert_name_state` null-info branch.
- `src-tauri/src/sql/028…031` — migrations.

**Frontend**
- `src/lib/auction.ts` — `auctionWindowText`, `pendingBroadcastText`,
  `pendingBroadcastBadge`, phase guide.
- `src/components/ui/Tooltip.tsx`, `src/components/name-actions/ActionHint.tsx`,
  `ActionReasonBanner.tsx`.
- `src/lib/rowClick.ts`, `src/components/ui/DataTable.tsx`,
  `src/components/WalletView.tsx`.
- `src/lib/nameSections.ts` — the stage matrix, and the only place that
  decides which sections exist.
- `src/components/name-actions/UpcomingSection.tsx`.

**Tests**
- `src-tauri/src/tests/live_node_it.rs` — the regtest end-to-end passes, gated
  on `HNS_IT_NODE_URL`. Run against a scratch node, never a chain you care
  about: they mine.
- `src-tauri/src/tests/{chain_scan,read_cmd,names_action_context,name_capabilities,names_cmd,deadlines_cmd}_tests.rs`.
- `src/lib/auction.test.ts`, `src/lib/rowClick.test.ts`,
  `src/lib/nameSections.test.ts`,
  `src/components/__tests__/{wallet-view,name-acquisition,auction-positions,tooltip}.test.tsx`,
  `src/components/name-actions/__tests__/name-bids-panel.test.tsx`.

**Docs**
- `docs/USER_MANUAL.md` — "Several bids on one name" (§8) and the amount line
  of the batch confirmation (§10).
- `docs/CODING_STANDARDS.md` — the no-native-`title` rule and the
  `lint:native-title` gate.
- `CHANGELOG.md` — `[Unreleased] / Fixed`, the entries from "Update,
  Transfer, Renew and Revoke are no longer offered…" down to "The locked-wallet
  notice carries an Unlock button, once."
