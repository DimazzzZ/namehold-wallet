---
area: Shakedex rows in Activity and on Owned Names, and the balance
priority: P1–P2
blocks: 0
requires:
  - the transactions the selling, buying and restore cases left on chain
last_verified: "eec6b2d (same tree as 150d58a) for C1 and C2; bc8cd4d for A1; 2026-10-18, report kept outside the repo"
---

# Activity and history

History rows are computed from the node on every read, so these cases need no new transactions: run them at the end of a run (or after a rebuild) over what the other cases left on chain. Labels and amounts are described in [ui.md](../../ui.md#activity-labels-for-shakedex).

## C1 — Shakedex labels survive the chain row

- **Priority:** P1
- **Was:** T7 fixes recheck C1; full QA "Activity: Buy, Finalize purchase, Lost" and the S2.3, S5.2, S5.4 label gaps
- **Covers:** R13, R19, R28
- **Regression of:** Q1 (Activity showed the generic `Transfer` / `Finalize` instead of the Shakedex labels once the chain row arrived), I1 (the Q1 fix dropped the `⤷ <value>` name-value line)
- **Preconditions:** mined Shakedex transactions made by the active profile: a purchase and its FINALIZE (B7, B8, B10), a lock, its FINALIZE into the lock, a cancel and its FINALIZE home (S2, R1, C4).

Steps:

1. Activity → search each name; for every row record the label, the status, the Amount and the `⤷` line.

Expected:

- A purchase: `Buy` (not `Transfer`); its buyer FINALIZE: `Finalize purchase` (not `Finalize`).
- A lock TRANSFER: `Lock for sale`; its FINALIZE into the lock: `Finalize into lock`; a cancel: `Cancel listing`; its FINALIZE home: `Finalize cancel`. All `Confirmed`.
- Rows that move a name show the `⤷ <name value>` line: `⤷ 0.500000` for a name won with two bids, `⤷ 0.000000` for a single-bid name.
- A row with no local draft (made in the other profile of the same phrase) shows the chain's generic label; that is not a finding. A label seen only before the block and the generic one after it is ❌ (Q1).
- Passes if every Shakedex row made here keeps its Shakedex label once mined. If no Shakedex label shows at all, check whether the drafts still exist; a pruned draft is ◐.

- **Blocks mined:** 0
- **Evidence:** one screenshot per name filter (`C1-<role>.png`), with the rows' labels quoted.
- **Cleanup / end state:** nothing changed.

## C2 — Purchase, cancel and finalize amounts

- **Priority:** P1
- **Was:** T7 fixes recheck C2; full QA B10 (Activity amount)
- **Covers:** R10, R13
- **Regression of:** Q3 (a purchase of a name with value showed the price plus the seller's name coin, `−5.500055` for a `5.000055` price)
- **Preconditions:** as C1; the Wallet balance noted before any new block.

Steps:

1. On the same rows as C1 read the Amount and the fee.
2. Spot-check two plain HNS receives or sends from earlier history.
3. Read the Wallet totals (available, Name Value, immature).

Expected:

- A purchase: Amount minus exactly the price paid (the dialog's `Price (to seller)`), the network fee shown separately; never the price plus the name's value.
- A seller's cancel, a FINALIZE home and a buyer's FINALIZE: Amount 0, with the `⤷` line.
- `Finalize into lock` of a name with value: record the Amount shown (one run saw minus the name's value, the coin leaving the wallet's own addresses for the lock); a zero-value name shows 0 with `⤷ 0.000000`.
- Plain sends and receives unchanged from what an earlier run recorded.
- The seller's sale rows: a green `+` the price received (direction received), never a gray unsigned amount.
- The Wallet totals unchanged by a rebuild alone (history rows do not feed the balance); any difference not explained by new blocks is ❌.
- Passes if no purchase or cancel row counts the name's value as money spent.

- **Blocks mined:** 0
- **Evidence:** screenshots of the rows; the Wallet totals before and after the rebuild.
- **Cleanup / end state:** nothing changed.

## A1 — Owned Names, My listings, Activity and the balance agree

- **Priority:** P2
- **Was:** full QA A1
- **Covers:** R14, R19, R22
- **Preconditions:** the end of each P1/P2 block of cases.

Steps:

1. For every name a case touched compare its Owned Names row, its My listings row and its Activity rows.
2. Add up what the cases paid and received and compare it with the Wallet totals.

Expected:

- The Owned Names badge equals the My listings label (the same N in `Locking · N blocks`).
- Purchase rows have no checkbox and no `Manage`; a Sold name shows `Sold` on Owned Names for a while and has no owner actions; a Cancelled or aborted name is an ordinary owned row.
- Activity rows carry the labels of [ui.md](../../ui.md#activity-labels-for-shakedex).
- The Wallet balance equals the run's coinbases plus sales, minus purchases and fees (a check from the account xpub and the node's address index is enough; never read the wallet database or the seed).
- Passes if nothing disagrees.

- **Blocks mined:** 0
- **Evidence:** screenshots `A1-1-owned-names`, `A1-2-activity`; the reconciliation table in the report.
- **Cleanup / end state:** nothing changed.
