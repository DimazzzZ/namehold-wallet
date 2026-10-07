# Honest broadcast result

Status: specified, not implemented. Merges after `feat/shakedex-buy` (PR 1 of Shakedex name sales), which adds `noncustodial::rpc::is_node_rejection` and changes the functions this spec touches.

## 1. Summary

When the user sends a transaction, the wallet says "sent" only once the node has the transaction. hsd 8.0.0's `sendrawtransaction` answers with the txid whatever its mempool does with the transaction, so today a send whose coins are already spent, or that the mempool turns away for any other reason, reads as broadcast and is quietly marked dropped ten minutes later. After this change the wallet asks the node right after sending; if hsd says it does not have the transaction, the dialog says "Not sent" and why it cannot say more, and the coins stay held until the node is checked again.

## 2. Terms

- **Taken by the node** — `getrawtransaction <txid>` answers the transaction: hsd looks in its mempool first and then in its chain (`node.getMeta`), so a transaction just accepted is found with or without a transaction index.
- **hsd's not-found** — hsd's own JSON-RPC answer to `getrawtransaction` for a transaction it does not have: `Transaction not found.`, code -1 (`RPCError(errs.MISC_ERROR, …)` in `lib/node/rpc.js`), sent with HTTP 200. Recognised by `rpc::is_node_rejection`; a proxy's page, a timeout or any reply that is not hsd's is not this answer.
- **Not taken** — every check in the window got hsd's not-found.

## 3. Requirements

**R1 — Sent means taken.** After `sendrawtransaction` returns a txid, the wallet checks the transaction with `getrawtransaction` up to 5 times, 400 ms apart (hsd adds the transaction to its mempool asynchronously after answering). The first check that finds it ends the window: the draft is `broadcasted` with that txid, as today.
*Enforced:* `commands/tx.rs::classify_broadcast_outcome_with_client`.
*Pinned:* `node_rpc_injected_tests::{broadcast_taken_by_the_node_is_success, broadcast_taken_on_a_later_check_is_success}`.

**R2 — Not taken is not sent, and the user is told.** When every check gets hsd's not-found, the broadcast returns an error and the draft becomes `broadcast_pending` with the note "The node did not take the transaction. hsd does not say why; often its coins are already spent elsewhere. You can try again; the coins stay held until the node is checked again." Its coin reservation is kept. The send dialog shows the error as it shows any failed send ("Not sent"), and a retry is allowed, as for any `broadcast_pending` draft.
*Enforced:* `commands/tx.rs::broadcast_tx_draft`.
*Pinned:* `node_rpc_injected_tests::broadcast_not_taken_by_the_node_is_pending`, `tx_lifecycle_tests::broadcast_not_taken_keeps_its_coins_and_says_why`.

**R3 — No answer is no verdict.** A check that fails in transport, or gets a reply that is not hsd's, ends the window without a verdict: the draft is `broadcasted`, since the node did return a txid. Only hsd's not-found on every check makes R2.
*Enforced:* `commands/tx.rs::classify_broadcast_outcome_with_client`.
*Pinned:* `node_rpc_injected_tests::broadcast_check_without_hsds_answer_is_success`.

**R4 — The draft lifecycle resolves it as before.** `refresh_tx_confirmations` promotes a `broadcast_pending` draft the node later knows, and after its grace window marks one hsd still does not know `failed` and releases its coins (unchanged).
*Enforced:* `commands/tx.rs::refresh_tx_confirmations`.
*Pinned:* `live_node_it::live_send_broadcast_double_spend_releases_reservation`, now deterministic: the second spend of a coin is not taken (`broadcast_pending`, the error returned), and is never mined.

**R5 — A Shakedex purchase follows the same rule.** The purchase broadcast goes through R1–R3. The one automatic rebroadcast (spec 2026-10-05 R13) uses the same check: a rebroadcast that is not taken loses the purchase with the existing REFUSED reason ("the node refused to take the purchase, so nothing was paid"), unless the chain still traces to it.
*Enforced:* `shakedex_jobs.rs::refresh_missing`.
*Pinned:* `shakedex_purchase_state_tests::rebroadcast_not_taken_is_lost`, `live_node_it::shakedex_cli_buyer_first_loses_ours_with_nothing_paid` (the purchase broadcast now returns the R2 error).

## 4. Explicitly not enforced

- **Why the node did not take it.** hsd logs the mempool's reason and does not return it; there is no `testmempoolaccept`. The wallet does not guess one.
- **What peers do.** For an orphan or a score-0 verification error hsd "broadcasts anyway" to its peers. A transaction the local node did not take is not proven unknown to the network; R2 keeps the coins held for that reason, and R4 decides later.

## 5. Known gaps

- **Mined inside the window on a node without a transaction index.** A transaction mined before the first check is found only through the index. On regtest with instant mining, or a remote node without `--index-tx`, such a send can read as not taken; R4 then marks it `failed` after the grace window, the gap the confirmation poll already has on such nodes.
- **Up to two seconds more per send** when the node does not have the transaction; a transaction the node takes is found on the first check.

## 6. Pointers

- `src-tauri/src/commands/tx.rs` (`classify_broadcast_outcome_with_client`, `broadcast_tx_draft`), `src-tauri/src/shakedex_jobs.rs` (`refresh_missing`).
- hsd 8.0.0: `lib/node/rpc.js` (`sendRawTransaction`, `getRawTransaction`), `lib/node/fullnode.js` (`relay`, `sendTX`, `getMeta`).
- `docs/USER_MANUAL.md` ("Send fails with Not sent"), `CHANGELOG.md`.
