# Honest broadcast result

Status: specified, not implemented. Builds on Shakedex name sales (merged) and on the chain evidence the confirmation poll reads for hsd's not-found (`fix/no-tx-index-send-verdicts`, `commands/tx.rs::chain_evidence_with_client`), which merges first. Its row in `docs/specs/README.md` is added with the implementation.

## 1. Summary

When the user sends a transaction, the wallet says "sent" only once the node has the transaction. hsd 8.0.0's `sendrawtransaction` answers with the txid whatever its mempool does with the transaction, so today a send whose coins are already spent, or that the mempool turns away for any other reason, reads as broadcast and is quietly marked dropped ten minutes later. After this change the wallet asks the node right after sending; if hsd says it does not have the transaction, the dialog says "Not sent" and why it cannot say more, and the coins stay held until the node is checked again.

## 2. Terms

- **Taken by the node** — `getrawtransaction <txid>` answers the transaction: hsd looks in its mempool first and then in its transaction index (`node.getMeta`), so a transaction just accepted is found with or without a transaction index. A transaction already mined is found only through the index; on a node without one, an output of the transaction that is a coin mined in a block (`chain_evidence_with_client` → `Mined`) is the same answer.
- **hsd's not-found** — hsd's own JSON-RPC answer to `getrawtransaction` for a transaction it does not have: `Transaction not found.`, code -1 (`RPCError(errs.MISC_ERROR, …)` in `lib/node/rpc.js`), sent with HTTP 200. Recognised by `rpc::is_tx_not_found`; a proxy's page, a timeout or any reply that is not hsd's is not this answer.
- **Not taken** — every check in the window got hsd's not-found.

## 3. Requirements

**R1 — Sent means taken.** After `sendrawtransaction` returns a txid, the wallet checks the transaction with `getrawtransaction` up to 5 times, 400 ms apart (hsd adds the transaction to its mempool asynchronously after answering). The first check that finds it ends the window: the draft is `broadcasted` with that txid, as today. A check that gets hsd's not-found also reads the chain evidence for the draft; `Mined` ends the window too, the draft `confirmed` at that height.
*Enforced:* `commands/tx.rs::classify_broadcast_outcome_with_client`.
*Pinned:* `node_rpc_injected_tests::{broadcast_taken_by_the_node_is_success, broadcast_taken_on_a_later_check_is_success, broadcast_mined_without_tx_index_is_success}`.

**R2 — Not taken is not sent, and the user is told.** When every check gets hsd's not-found, the broadcast returns an error and the draft becomes `broadcast_pending` with the note "The node did not take the transaction. hsd does not say why; often its coins are already spent elsewhere. You can try again; the coins stay held until the node is checked again." Its coin reservation is kept. The send dialog shows the error as it shows any failed send ("Not sent"), and a retry is allowed, as for any `broadcast_pending` draft.
*Enforced:* `commands/tx.rs::broadcast_tx_draft`.
*Pinned:* `node_rpc_injected_tests::broadcast_not_taken_by_the_node_is_pending`, `tx_lifecycle_tests::broadcast_not_taken_keeps_its_coins_and_says_why`.

**R3 — No answer is no verdict.** A check that fails in transport, or gets a reply that is not hsd's, ends the window without a verdict: the draft is `broadcasted`, since the node did return a txid. Only hsd's not-found on every check makes R2.
*Enforced:* `commands/tx.rs::classify_broadcast_outcome_with_client`.
*Pinned:* `node_rpc_injected_tests::broadcast_check_without_hsds_answer_is_success`.

**R4 — The draft lifecycle resolves it as before.** `refresh_tx_confirmations` promotes a `broadcast_pending` draft the node later knows or finds mined by its outputs, and after its grace window resolves one hsd still does not know by the chain evidence (unchanged): every coin it spends unspent marks it `failed` and releases its coins; a coin it spends spent by another transaction, on a node with a transaction index, marks it `dropped` saying so and releases its coins; otherwise it stays held.
*Enforced:* `commands/tx.rs::refresh_tx_confirmations`.
*Pinned:* `live_node_it::live_coins_spent_by_another_tx_drop_the_draft_and_say_so`, changed to expect the R2 error from the second spend of a coin (`broadcast_pending`), then `dropped` after the grace window.

**R5 — A Shakedex purchase follows the same rule.** The purchase broadcast goes through R1–R3. The one automatic rebroadcast (spec 2026-10-05 R13) uses the same check: a rebroadcast that is not taken loses the purchase with the existing REFUSED reason ("the node refused to take the purchase, so nothing was paid"), unless the chain still traces to it.
*Enforced:* `shakedex_jobs.rs::refresh_missing`.
*Pinned:* `shakedex_purchase_state_tests::rebroadcast_not_taken_is_lost`, `live_node_it::shakedex_cli_buyer_first_loses_ours_with_nothing_paid` (the purchase broadcast now returns the R2 error).

## 4. Explicitly not enforced

- **Why the node did not take it.** hsd logs the mempool's reason and does not return it; there is no `testmempoolaccept`. The wallet does not guess one.
- **What peers do.** For an orphan or a score-0 verification error hsd "broadcasts anyway" to its peers. A transaction the local node did not take is not proven unknown to the network; R2 keeps the coins held for that reason, and R4 decides later.

## 5. Known gaps

- **Mined inside the window on a node without a transaction index, its outputs already spent.** R1 finds a mined transaction on such a node only through an output that is still a coin. One whose every output was spent within the window reads as not taken, and R4 leaves it held.
- **Up to two seconds more per send** when the node does not have the transaction; a transaction the node takes is found on the first check.

## 6. Pointers

- `src-tauri/src/commands/tx.rs` (`classify_broadcast_outcome_with_client`, `broadcast_tx_draft`), `src-tauri/src/shakedex_jobs.rs` (`refresh_missing`).
- hsd 8.0.0: `lib/node/rpc.js` (`sendRawTransaction`, `getRawTransaction`), `lib/node/fullnode.js` (`relay`, `sendTX`, `getMeta`).
- `docs/USER_MANUAL.md` ("Send fails with Not sent"), `CHANGELOG.md`.
