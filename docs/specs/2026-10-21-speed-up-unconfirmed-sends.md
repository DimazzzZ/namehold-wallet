# Speed up an unconfirmed send

**Status:** accepted (2026-10-21). Amends [2026-10-05 Shakedex name sales](./2026-10-05-shakedex-name-sales.md) R13 (the purchase's change) and replaces its §5 gap "No fee bump". Builds on [2026-10-07 Honest broadcast result](./2026-10-07-honest-broadcast-result.md) (a send counts as sent only once the node has it).

## 1. Summary

A transaction that pays too little for the blocks it competes with can sit in the mempool for up to 72 hours and then vanish. hsd 8.0.0 has no replace-by-fee: a second transaction spending a coin already spent in the mempool is refused as `bad-txns-inputs-spent` (`mempool/mempool.js:1551-1557`, `isDoubleSpend` at `mempool/mempool.js:2429-2437`), so the only way to help it is a child that spends its change and pays for both (child pays for parent, CPFP). hsd's own block template takes a transaction at the better of its own fee rate and the rate of it with all its descendants (`cmpRate`, `mining/miner.js:641-657`; `BlockEntry.fromEntry`, `mining/template.js:665-672`), so a well-paying child pulls its parent forward. This spec adds "Speed up" to every unconfirmed transaction of ours that has a change output we can spend, Shakedex purchases included: one click builds that child, sized so the parent and its descendants together reach a target fee rate, and the confirmation says plainly what it costs and what it does not do.

hsd paths in this spec are relative to `lib/` of hsd 8.0.0, the version every protocol fact here is pinned to.

## 2. Terms

- **Parent** — the unconfirmed transaction of ours being sped up. Its txid does not change: the child is a separate transaction, and hsd stores the parent's entry as it is (`mempool/mempoolentry.js:94-127`).
- **Child** — the transaction Speed up builds: it spends the parent's change and pays the fee for the package.
- **Package** — the parent and every one of its mempool descendants, ours or not. hsd keeps their totals on the parent's entry (`descFee`, `descSize`, `mempool/mempoolentry.js:123-124`, raised for every new descendant in `addEntry`, `mempool/mempool.js:1808`, and lowered on eviction, `mempool/mempool.js:1852-1856`), and `getmempoolentry` returns them as `descendantsize` (vbytes) and `descendantfees` (doos, while its `fee` is in HNS) (`node/rpc.js:3098-3114`).
- **Package rate** — `descendantfees` over `descendantsize`, in hsd's arithmetic `getRate(size, fee) = floor(fee * 1000 / size)` doos per kB (`protocol/policy.js:240-248`); shown to the user in doos per vbyte as the send form shows a rate.
- **Size** — hsd's sigop-adjusted virtual size, `ceil(max(weight, sigops * 20) / 4)` (`mempool/mempoolentry.js:96-97`, `primitives/tx.js:225-230`). For the P2WPKH inputs a child has, the weight term dominates and this is the vsize (`primitives/tx.js:213-216`).

## 3. Requirements

Requirements name the code that enforces them and the test that pins them; every name is to be given by the implementation and checked by `scripts/check-spec-refs.sh`.

### 3.1 Which transactions can be sped up

**R1 — A parent is ours, sent, unconfirmed and in the node's mempool.** Speed up is offered for a draft of this profile that is `broadcasted` (taken by the node, Honest broadcast result R1), not confirmed, and that the profile's node returns from `getmempoolentry`. hsd answers a transaction it does not hold, mined or not, with `Transaction not in mempool.` (`node/rpc.js:875-893`): that is a refusal, never a child built on a guess. A parent the node does not have would leave the child an orphan, kept by hsd only until its parent arrives (`maybeOrphan`, `mempool/mempool.js:1563-1568, 2222-2253`; at most 100, `protocol/policy.js:142`), and there is no package relay; the remedy for a missing parent is a rebroadcast, not a child. A `broadcast_pending` draft (the node did not take it) is not offered. A draft already `confirmed`, or found mined by the confirmation poll, is not offered. The node must be a local node or a remote node that can send (the same gates as any broadcast, `broadcast_gates`); in SPV and Explorer modes, which have no `getmempoolentry`, Speed up is not offered and the row says it needs a full or remote node.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R2 — A parent has a change output of ours that nothing else spends.** The child's only input is the parent's change, the output at `DraftPlan.change_output_index`; a plain send stores its build parameters rather than a plan, and its change is output 1, after the recipient (`send.rs::build_send`). A parent without one (a purchase whose change fell below `DUST_THRESHOLD` into the fee, a send of everything, a covenant draft whose outputs are all name outputs) is refused with "This transaction has no change to spend, so it cannot be sped up." A covenant output is never spent into a child: a name output is not ours to move with a plain send, and a purchase's TRANSFER sits at the seller's lock address, where only a FINALIZE may spend it (spec 2026-10-05 R13). The change must be in `tracked_utxos`, unspent, covenant-free and not reserved by another draft; a change already spent by another transaction of ours (an ordinary send that chose it, since unconfirmed ordinary change is selectable today) is refused with "Its change is already spent by another of your transactions, which pays its own fee."
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R3 — One child per parent.** A parent whose change is spent or reserved by a live child (any status but `dropped`, `failed` or deleted) is refused with "This transaction is already being sped up." The child itself is not offered Speed up: its row says which transaction it speeds up. hsd would accept a chain (the ancestor limit is 50, `protocol/policy.js:117`, enforced at `mempool/mempool.js:1673-1678`, and there is no descendant limit: `countDescendants`, `mempool/mempool.js:1980`, is never checked in `verify`), but a second bump is left out of this spec (§5).
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R4 — A purchase is sped up only while it is an Unconfirmed purchase.** For a Shakedex purchase the purchase job's state decides, not the draft's status: Speed up is offered only while the `shakedex_purchases` row is `unconfirmed` (spec 2026-10-05 R13). `pending_send`, awaiting finalize, owned and both lost states are refused. A purchase that lost the race for the lock coin elsewhere is refused by every node that holds the winner (§1's double-spend rule), so a child raises its odds only where our purchase is held; once the job reads it lost, no child is offered. A purchase whose paid step is not final yet is not in any mempool (`mempool/mempool.js:1518`) and R1 refuses it already.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R5 — Not close to the mempool expiry.** hsd forgets a transaction without a mempool parent 72 hours after it entered that node's mempool (`MEMPOOL_EXPIRY_TIME`, `protocol/policy.js:134`), and evicts its spenders with it (`limitSize`, `mempool/mempool.js:619-637`; `evictEntry`/`removeSpenders`, `mempool/mempool.js:1852-1878`). The clock is the parent's `entry.time`, set when it entered this mempool (`mempool/mempoolentry.js:119`) and returned by `getmempoolentry` as `time`; a child does not reset it. A parent with less than 6 hours left of `time + 72 h`, by the node's own clock (`getblockchaininfo` gives none; the wallet compares with its own clock and says so), is refused with "This transaction leaves your node's mempool in less than 6 hours; speeding it up now would not keep it there." The 6-hour margin is a choice of this spec, not an hsd rule.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R6 — A parent paying below the relay floor is refused.** hsd's floor is `MIN_RELAY = 1000` doos per kB on every network (`protocol/policy.js:65`; `protocol/networks.js:589, 797, 962, 1131`), `getMinFee(size) = floor(rate * size / 1000)` (`protocol/policy.js:188-203`). A transaction below it can still sit in the mempool as "free" on coin-age priority (`mempool/mempool.js:1638-1666`), and hsd's miner skips a free parent once the block holds `MIN_BLOCK_WEIGHT` (5000, `protocol/policy.js:152`) whatever its child pays, because `free` looks at the parent's own fee only (`mining/miner.js:412-415`, `mining/template.js:670`). The wallet never builds such a transaction (plain sends floor at 1 doo/vB, Shakedex at 5), but a parent whose `getmempoolentry` fee is below `getMinFee(size)` is refused with "This transaction pays less than the network's minimum relay fee; a child cannot pull it into a block."
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

### 3.2 The child

**R7 — One input, one output.** The child spends exactly the parent's change outpoint `(parent txid, change_output_index)`, final sequence, lock time 0, and pays everything left after its fee to the profile's change address, change branch index 0, where every send's change goes (`commands/tx.rs::change_address`, `draft_ctx::load_ctx`) and the only change output a Ledger accepts (`verify_ledger_change_output`). It pays no one else and adds no other coin of ours: a confirmed coin added to it would share the parent's fate (§5). The builder names the outpoint directly and never goes through ordinary coin selection (`send.rs::load_spendable_coins`), so the purchase hold of R13 does not refuse it (R12). The child's draft reserves that one outpoint in the same database transaction that writes it (`insert_tx_draft_reserving_coins_in_tx`); a reservation that cannot be claimed refuses the build. Its plan records the output as its change (`change_output_index` 0) and passes `check_standard_weight`.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R8 — The fee lifts the whole package to the target rate, counted on hsd's sizes.** Let `R` be the target in doos per vbyte, `S_c` the child's size, and `descendantsize`/`descendantfees` the parent's from `getmempoolentry` at build time; the package already counts every descendant, ours or someone else's (a seller spending the payment output, for example), as hsd's descendant rate does. The child's fee is `max(ceil(R * (descendantsize + S_c)) - descendantfees, S_c)`: the second term is the child's own relay floor, because hsd checks each transaction's own fee against the floor and has no package acceptance (`mempool/mempool.js:1633`). `S_c` is measured on the signed child (about 108 vbytes for one P2WPKH input and one output, the wallet's estimator pinned against hsd by `send.rs::estimate_size_matches_hsd_vsize`), and the package rate after it is checked again in hsd's arithmetic (`getRate`) before the draft is stored: below `R` the build is refused, not rounded. The draft's summary stores the parent txid, the package rate before and after, the child's fee, and the parent's expiry deadline, so the confirmation and Activity read the build's numbers, not a later node read.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R9 — The target rate is chosen as the send form chooses it.** By default `R` is the node's estimate, `resolve_fee_rate` as a send uses it (`estimatesmartfee`, `node/rpc.js:2287-2307`, a median of observed confirmation rates, `mempool/fees.js:673-700`; with no data hsd returns -1, `node/rpc.js:2299-2300`, and the wallet falls back to 1 doo/vB). The user may type a rate instead, as in the send form. The estimator ignores transactions with a mempool parent and records each one's own rate (`mempool/fees.js:543-547`), so it is a fair target for a package but knows nothing of CPFP. A target at or below the current package rate changes nothing and is refused with "Your transaction already pays this rate or more; choose a higher one." When the estimate is the fallback, the form says the node has no fee estimate yet.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R10 — Dust and the absurd-fee cap.** The build is refused when the change left after the fee is below `DUST_THRESHOLD` (1000 doos, `send.rs`) with "The change of this transaction is too small to pay for speeding it up", and when the child's fee exceeds hsd's absurd-fee cap, `getMinFee(S_c) * 10000` (`ABSURD_FEE_FACTOR`, `protocol/policy.js:73`; checked at `mempool/mempool.js:1636, 1669-1670`, on by default, `mempool/mempool.js:2776`), about 1.08 HNS for a 108-vbyte child, with "This would pay more in fee than your node accepts from one transaction (about N HNS); choose a lower rate." Both are checked before the confirmation and again on the signed child.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

### 3.3 Confirmation, signing and broadcast

**R11 — The confirmation says what is paid and what is not changed.** The secure confirmation lists: the fee the child pays, in HNS; the package rate before and after, in doos per vbyte, against the target and its source (the node's estimate or the typed rate); that the fee goes to the network only and nobody else is paid (for a purchase: the seller's price and the purchase itself are unchanged); that the original transaction keeps its transaction id; and that it does not extend the time the network keeps the original transaction waiting: "Speeding up does not extend the 72 hours your node keeps the original transaction: if it is not mined by <deadline>, both are dropped and nothing is paid by either." The deadline is the parent's `time + 72 h` stored at build time. Nothing is signed unless it is confirmed.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R12 — The purchase hold stays, and the child is its one exception.** Spec 2026-10-05 R13 says "Until confirmed, the purchase's change is excluded from coin selection" (`send.rs::load_spendable_coins` excludes outputs of a purchase in `pending_send` or `unconfirmed`), because a purchase that loses the race never existed and takes every spender with it. That rule stays for ordinary coin selection, and is amended to: until confirmed, the purchase's change is spent only by its Speed up child, which names that outpoint itself (R7); and the child's own output is held back from coin selection the same way, until the purchase is confirmed, since it falls with the purchase. The FINALIZE path is untouched: it follows the purchase's output 0 and spends `transfer_outpoint` (`shakedex_jobs.rs`, `noncustodial/shakedex/purchase.rs`), and the child spends neither.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation (including an amendment of `shakedex_purchase_store_tests::change_of_unconfirmed_purchase_is_not_spendable` and the live `shakedex_purchase_change_is_held_back_until_seen_mined` to cover the child's output).

**R13 — Signed from its stored plan, on every signer the parent could use.** The child is a stored `DraftPlan`, signed by `sign_plan` like every action but `send_hns`, which re-selects coins at sign time and could not keep the one input. For a Ledger profile the plan goes to the device as any covenant-free stored plan: `refuse_unsupported_ledger_plan` refuses Shakedex actions by their `shakedex_` prefix and plans with a foreign or lock-key input, a non-final sequence or a lock time (`DraftPlan::has_foreign_or_custom_inputs`), none of which the child has, and its action is not a `shakedex_` one; a Ledger profile has no purchases (spec 2026-10-05 R16), so a Ledger child always speeds up an ordinary transaction. This is from reading `commands/tx.rs::{refuse_unsupported_ledger_plan, sign_via_ledger}` and `noncustodial/actions.rs`; no Ledger has signed a child yet. Watch-only profiles cannot sign and are not offered Speed up.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation (a test that the Ledger guard passes the child's plan).

**R14 — Broadcast re-checks the parent.** Broadcasting the child goes through `broadcast_tx_draft` and Honest broadcast result R1–R3, and first reads `getmempoolentry` for the parent again: a parent no longer in the node's mempool (mined meanwhile, or gone) refuses the broadcast and drops the unsent child with "The transaction it speeds up is no longer waiting in your node's mempool; nothing was sent." For a purchase, the purchase row must still be `unconfirmed`.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

### 3.4 Tracking and where it is shown

**R15 — The child is a draft like any other, and says why it died.** The child is followed by `refresh_tx_confirmations` as any draft. When the parent dies (dropped from the mempool at its expiry, a purchase lost, the parent's coins spent elsewhere), the child's input never existed, which the chain evidence reads as `InputsSpent` (`noncustodial/tx_evidence.rs`) and the lifecycle today words as "Another transaction spent this transaction's coins, so it can never confirm." For a child that sentence names the wrong cause; it is dropped instead with "The transaction it sped up (<parent txid>) did not confirm, so this one can never confirm. Nothing was paid by either." When the parent is a purchase the purchase job has given up, the child is dropped in the same database transaction that loses the purchase, with the same sentence. When the purchase job rebroadcasts a purchase (spec 2026-10-05 R13, once, after 6 blocks missing), it sends the live child right after it, through the same taken-by-the-node check; a child the node does not take stays `broadcast_pending` and is judged by the lifecycle.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

**R16 — Where the button is.** Activity: on every row whose draft is `broadcasted` and unconfirmed, a "Speed up" action; refused rows show it disabled with their reason (R1–R6), never hidden. A child's row reads "Speeds up <short parent txid>", and the parent's row, while a child is live, "Sped up by <short child txid>". The Market's Your purchases card: on an Unconfirmed purchase, the same "Speed up", opening the same form and confirmation. The form shows the parent's size, fee and package rate, the target rate (estimate or typed), the child's fee and what is left of the change, and the mempool deadline.
*Enforced:* to be named by the implementation.
*Pinned:* to be named by the implementation.

### 3.5 Live node

**R17 — hsd's template takes the parent at its package rate.** The source says that `getblocktemplate` (`miner.createBlock`, `node/rpc.js:1408-1409, 2718`; `assemble`, `mining/miner.js:121-157, 330-362, 425-433`) queues a parent at `max(rate, descRate)`, which is what makes a child worth sending; no run has shown it yet. A live regtest test shows it on hsd 8.0.0: a node started with a small `--max-weight` (`node/fullnode.js:140`, `mining/miner.js:478, 562`) so a block holds only a few transactions; a 1 doo/vB parent of ours and enough transactions at a higher own rate to fill the block without it; `getblocktemplate` leaves the parent out; after the wallet's child is broadcast, `getblocktemplate` includes the parent and the child, and the next mined block confirms both. The same suite covers the refusals that rest on hsd's replies: a parent not in the mempool, a mined parent, a child whose fee would pass the absurd cap, and the child dropped with R15's sentence when the parent is evicted (`invalidateblock` empties the mempool and is not a fair stand-in for expiry; the test removes the parent by mining a conflicting spend of one of its inputs on a second node and connecting the nodes).
*Enforced:* —
*Pinned:* to be named by the implementation (in `tests/live_node_it.rs`, env-gated like the rest of the live suite).

## 4. Explicitly not enforced

- A child helps only miners who build blocks with hsd's descendant-rate ordering. Whether Handshake pools mine with hsd's template is not something the wallet can see.
- A child does not keep the parent alive past its 72 hours, and does not help a parent the network does not have: other nodes hold the child as an orphan at most.
- The fee estimate is the node's; the wallet does not second-guess it.

## 5. Known gaps

- **One bump only.** A second Speed up of the same parent, or of the child, is refused (R3). A chain is valid in hsd; if the first target proves too low, the user waits or lets the parent expire and sends again.
- **No other coins.** The child spends only the parent's change (R7). A parent whose change is too small to pay for the package (R10) cannot be sped up, though a child adding a confirmed coin could.
- **No change, no speed up.** A purchase whose change fell into its fee, a send of everything and a covenant draft without change cannot be helped (R2).
- **The package counts descendants that are not ours.** A foreign descendant that pays well lowers the fee R8 asks; one that pays badly raises it. The numbers are the node's at build time and can move before the child is mined.
- **Practical value is modest on hsd.** Anything the mempool accepted already pays the relay floor, so a transaction is stuck only while blocks are full by weight or by the per-block covenant caps (opens 300, updates 600, renewals 600, `protocol/consensus.js:195, 203, 211`, checked at `mining/miner.js:388-401`; a purchase's TRANSFER counts as an update, `covenants/rules.js:604-628`). The more common failure, a transaction absent from other mempools, is a rebroadcast's job, not a child's.
- **The expiry is the profile's node's.** R5 reads `time` from the node the wallet talks to; another node may have taken the parent earlier and forget it sooner.
- **Ledger signing of a child is untested on a device** (R13).

## 6. Pointers

- hsd 8.0.0: `mempool/mempool.js` (double spend, relay floor, absurd fee, ancestor limit, descendant totals, expiry, orphans), `mempool/mempoolentry.js` (`size`, `descFee`, `descSize`, `time`, `getDescRate`), `mining/miner.js` and `mining/template.js` (the template's ordering and the free-parent limit), `protocol/policy.js` (`MIN_RELAY`, `ABSURD_FEE_FACTOR`, `MEMPOOL_EXPIRY_TIME`, `MIN_BLOCK_WEIGHT`), `node/rpc.js` (`getmempoolentry`, `estimatesmartfee`, `getblocktemplate`).
- Wallet code this builds on: `noncustodial/actions.rs` (`DraftPlan`, `rebuild_unsigned`, `sign_plan`), `noncustodial/shakedex/funding.rs` (`own_input`, the signed-vsize measurement), `noncustodial/send.rs` (`DUST_THRESHOLD`, `check_standard_weight`, `select_all_coins` as the one-in-one-out shape, `load_spendable_coins` and its purchase hold), `db/queries.rs::insert_tx_draft_reserving_coins_in_tx`, `commands/tx.rs` (`resolve_fee_rate`, `broadcast_tx_draft`, `refresh_tx_confirmations`, `refuse_unsupported_ledger_plan`), `noncustodial/tx_evidence.rs`, `noncustodial/node_rpc.rs` (a new `getmempoolentry` call beside its siblings), `shakedex_jobs.rs` (the purchase job's rebroadcast), `src/components/ActivityView.tsx` (broadcasted rows), `src/components/market/PendingPurchases.tsx` and `src/lib/auction.ts` (the Unconfirmed purchase label).
- Spec 2026-10-05 R13's sentence on the purchase's change and its §5 "No fee bump" change with this spec; `docs/USER_MANUAL.md`, `docs/QA_WORKFLOWS.md` and `CHANGELOG.md` describe Speed up when it lands.
