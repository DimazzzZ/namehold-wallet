# Testing the non-custodial wallet on regtest

The wallet signs locally and only talks to a node for **reads + broadcast**. To
exercise everything end-to-end you need a local **hsd regtest node with the
address index enabled** (so `getcoinsbyaddress` works).

## TL;DR — automated live suite (canonical)

For an automated, end-to-end check that our tx-building/covenant code is
actually accepted on-chain, use the runner instead of the manual dance below:

```sh
bash scripts/regtest.sh run-it
```

This boots a disposable regtest node (idempotent), points the live-node
integration suite at it, and runs every `live_*` test in
`src-tauri/src/tests/live_node_it.rs` green (send, full auction lifecycle,
single transfer→finalize, and batch transfer). The node's data dir is the
repo-local, git-ignored `.regtest/` — it never touches your real `~/.hsd` chain.

Other subcommands:

```sh
bash scripts/regtest.sh start            # launch node, wait for RPC (idempotent)
bash scripts/regtest.sh fund <addr> 110  # mine 110 blocks to <addr>
bash scripts/regtest.sh mine 1 <addr>    # advance the chain one block
bash scripts/regtest.sh rpc getnameinfo <name>   # hsd-cli passthrough
bash scripts/regtest.sh stop             # graceful shutdown
bash scripts/regtest.sh reset            # stop + wipe .regtest/ for a fresh chain
```

Mining 110 blocks is about having enough value to spend, not about maturity:
on regtest `coinbaseMaturity` is **2** blocks (mainnet and testnet 100, simnet
6), so the first coinbase is spendable almost immediately.

With no node/env set, the same tests print "skip" — so `cargo test` /
`cargo nextest` stays offline and CI is unaffected. See the `namehold-qa`
skill for details.

> **Where data lives.** The regtest *chain* data is the repo-local, git-ignored
> `.regtest/` dir (via `hsd --prefix`); a manual `hsd --network=regtest` without
> `--prefix` uses `~/.hsd/regtest/` instead. The *wallet's* own state (profiles,
> portfolio, and the regtest node RPC config you enter in Settings) is not
> per-network — it lives in the single shared `~/.namehold/portfolio.db`. See
> "Where data and node config live" in `NODE_SETUP.md`.

The rest of this doc is the **manual** walkthrough (useful for exercising the
UI by hand); `scripts/regtest.sh` automates steps 1 and 4 for you.

## 1. Start a regtest node

Install hsd if needed (`npm i -g hsd`, or build from source), then:

```sh
hsd --network=regtest \
    --index-address --index-tx \
    --http-host=127.0.0.1 --api-key=test \
    --no-wallet
```

- Node RPC is now at `http://127.0.0.1:14037` (regtest), API key `test`.
- `--index-address` is **required** (the wallet scans coins by address); both indexes
  must be set from the first sync — hsd can't add an index to an existing chain.
- `--no-wallet` is fine — we are non-custodial; we never use hsd's wallet.

Helper CLI (separate terminal): `hsd-cli --network=regtest --api-key=test rpc <method> [args]`.

## 2. Launch the app

```sh
cd ~/git/namehold-wallet
pnpm install        # first time only
pnpm tauri dev
```

## 3. Configure + create a wallet

1. Onboarding → **Create a new wallet**, network **regtest**. A separate
   **secure window** asks for a passphrase, then shows your recovery phrase —
   confirm backup. (The main React UI never sees the phrase.)
2. Settings → **Node RPC**: URL `http://127.0.0.1:14037`, API key `test`,
   chain source **Local node**.

## 4. Fund it (mine regtest coins to your receive address)

Copy the **Receive Address** from the Wallet page, then mine to it. On
regtest a coinbase matures after **2** blocks (hsd `coinbaseMaturity`; it is
100 on mainnet/testnet), so mine a comfortable 110 to have plenty of value:

```sh
hsd-cli --network=regtest --api-key=test rpc generatetoaddress 110 <receiveAddress>
```

Back in the app, click **Sync** → the spendable balance should appear.

## 5. Plain send

Wallet → **Send HNS** → address + amount → **Review** (fee/change preview) →
**Sign & Broadcast** (unlock in the secure window if locked). Then:

```sh
hsd-cli --network=regtest --api-key=test rpc generatetoaddress 1 <anyAddress>
```

Sync again; the tx shows under Recent transactions.

## 6. Name auction (acquire a fresh name)

In the Owned Names box, type a test name (e.g. `testname`) → **Name actions**:

1. **Open** → broadcast → mine `treeInterval`+1 blocks (regtest treeInterval=5):
   `generatetoaddress 6 <addr>`.
2. **Bid** (e.g. bid `1000000`, lockup `2000000`) → mine through the bidding
   period (regtest biddingPeriod=5): `generatetoaddress 6 <addr>`.
3. **Reveal** → mine through reveal period (regtest revealPeriod=10):
   `generatetoaddress 11 <addr>`.
4. **Sync**, then **Register** (optionally paste records JSON) → mine 1 block.

Verify on the node: `hsd-cli --network=regtest --api-key=test rpc getnameinfo testname`.

## 7. Migration lifecycle (a name you already own)

For a name the wallet owns (after Register, or after a transfer to you):
**Manage** → **Update** (records JSON), **Renew**, **Transfer** (to another
address) → mine `transferLockup` blocks (regtest=10) → **Finalize**. **Cancel**
reverts a pending transfer; **Revoke** burns the name.

Records JSON example for Update/Register:

```json
[{"type":"TXT","txt":["hello world"]},{"type":"NS","ns":"ns1.example."}]
```

## Why regtest is the authoritative E2E layer

Unit tests and mockito-backed integration tests validate **logic correctness in isolation** — they prove that the right RPC calls are made, the right DB rows are written, and the right errors are returned for bad inputs. But they cannot prove that:

- A transaction built by our covenant/serialization code is **accepted by hsd's mempool**
- A bid coin is correctly matched during reveal by the real chain state
- A TRANSFER covenant is finalized after the lockup period expires
- The address index (`getcoinsbyaddress`) returns the UTXOs we expect
- Block confirmations advance as expected and trigger the right state transitions

**Only regtest validates on-chain acceptance.** Treat it as the final gate before any release:

1. **Unit tests** — pure logic, no I/O
2. **Integration tests** — mock DB + mockito RPC, prove orchestration
3. **Regtest** — real hsd, real chain, real broadcast, real confirmation

If a flow passes unit + integration tests but fails on regtest, the regtest result is authoritative.

---

## Notes / known caveats

- Covenant **serialization + signing** match hsd v6.1.1 byte-for-byte and are
  unit-tested, but on-chain acceptance of each action is exactly what this
  regtest pass validates — start here before testnet/mainnet.
- If `getcoinsbyaddress` errors, the node was started without `--index-address`.
- REVEAL/REDEEM need the wallet to have **synced** after the BID so it can find
  the bid coin (it matches by the bid address).
- Remote-node broadcast is gated behind `allow_remote_broadcast`; local node
  broadcasts freely.

---

## Mandatory Auction-Lifecycle Validation Scenarios

The following scenarios must be validated against a running regtest node to confirm the full auction lifecycle works end-to-end.

### Prerequisites

1. Start a regtest hsd node:
   ```bash
   hsd --network=regtest --index-address --index-tx --api-key=testkey --listen
   ```
   Node RPC listens on `127.0.0.1:14037` (regtest's default), NOT the mainnet
   port 12037.

2. Mine an initial set of blocks so names can be opened:
   ```bash
   hsd-rpc --network=regtest generatetoaddress 200 $(hsd-rpc --network=regtest getnewaddress)
   ```

3. Start the Namehold app with the regtest node configured in Settings.

### Scenario 1: Winner — Open → Bid → Reveal → Register

| Step | Action | Expected Result |
|------|--------|----------------|
| 1 | Open a name (e.g. `newname`) via `NameActionsModal` or `build_open_draft` | OPEN tx broadcasted; phase transitions to `OPENING` |
| 2 | Mine 10+ blocks until BIDDING phase | Phase shows `BIDDING`; countdown shows blocks until reveal |
| 3 | Place a blind bid with `bid > 0` and `lockup ≥ bid` | BID tx broadcasted; bid commitment persisted locally |
| 4 | Mine 10+ blocks until REVEAL phase | Phase shows `REVEAL`; `capabilities.canReveal` is true |
| 5 | Reveal the bid | REVEAL tx broadcasted; reveal coin created |
| 6 | Mine 10+ blocks until CLOSED phase | Phase shows `CLOSED`; task state is `wonNeedsRegister` |
| 7 | Register the name with DNS records | REGISTER tx broadcasted; name ownership finalized |
| 8 | Verify `capabilities.ownsName` is `true` | Backend confirms wallet controls the name |

### Scenario 2: Loser — Open → Bid → Reveal → Redeem

| Step | Action | Expected Result |
|------|--------|----------------|
| 1 | Open a name | OPEN tx broadcasted |
| 2 | Mine to BIDDING phase | Phase transitions |
| 3 | Place a bid | BID tx broadcasted |
| 4 | Mine to REVEAL phase | Phase transitions |
| 5 | Reveal the bid | REVEAL tx broadcasted |
| 6 | Have another wallet place a higher bid and complete the auction | Other wallet wins the name |
| 7 | Phase shows `CLOSED`; task state is `lostNeedsRedeem` | `capabilities.canRedeem` is `true` |
| 8 | Redeem the reveal coin | REDEEM tx broadcasted; funds reclaimed |

### Scenario 3: Transfer → Finalize

| Step | Action | Expected Result |
|------|--------|----------------|
| 1 | Register a name (from Scenario 1) | Name is owned |
| 2 | Transfer the name to another regtest address | TRANSFER tx broadcasted |
| 3 | Finalize the transfer right away | Refused: "the transfer of '…' is still locked for N more blocks" |
| 4 | Mine the transfer lockup (10 blocks on regtest), then finalize | FINALIZE tx broadcasted; name moves to recipient |
| 5 | Verify original wallet no longer `ownsName` | `capabilities.ownsName` is `false` |

### Scenario 4: Missed Reveal (simulated)

| Step | Action | Expected Result |
|------|--------|----------------|
| 1 | Open a name and bid | BID tx broadcasted |
| 2 | Mine through REVEAL phase without revealing | Phase transitions to CLOSED |
| 3 | Verify task state is `lostNeedsRedeem` | `canRedeem` is `true`; bid lockup still reclaimable |
| 4 | Redeem the lockup | Lockup value returned to wallet |

### Backend Capability Verification

Run the following command to validate the capability model:
```bash
cd src-tauri && cargo test -- auction_capabilities
```

Expected: 25 tests pass covering all task-state derivations and next-action mappings.

### Full Backend Test Suite

```bash
cd src-tauri && cargo test
```

This runs all unit tests including the capability tests, query tests, and other module tests. Integration tests against a live node require additional setup.

## Auto-Run Lifecycle Tests

The regtest lifecycle tests are gated behind the `HNS_IT_NODE_URL` environment variable so
they are skipped during normal `cargo test`. To run them against a local regtest node:

```bash
# Start a regtest hsd node first (see section 1 above), then:
HNS_IT_NODE_URL=http://127.0.0.1:14037 \
HNS_IT_NODE_API_KEY=test \
  cargo test --manifest-path src-tauri/Cargo.toml live_node -- --nocapture --test-threads=1
```

This runs the following tests:

| Test | Lifecycle |
|------|-----------|
| `live_auction_open_bid_reveal_register` | Winner: OPEN → BID → REVEAL → REGISTER |
| `live_auction_open_bid_reveal_redeem` | Loser redeem: OPEN → BID, a rival wallet on the same node bids higher → both REVEAL → REDEEM; the redeem spends exactly the losing reveal, returns its value, and the rival's reveal stays the owner |
| `live_auction_register_transfer_finalize` | Post-win: REGISTER → TRANSFER → FINALIZE refused inside the lockup → mine the rest of it → FINALIZE; the owner coin is at the recipient and `transfer` is 0 |
| `live_batch_transfer_two_names` | Batch: acquire 2 → BATCH TRANSFER → lockup → FINALIZE each, syncing in between; both owner coins are at the recipient |

### Send-path money invariants (Group A)

| Test | Asserts |
|------|---------|
| `live_send_builds_broadcasts_and_confirms` | Baseline: build → sign → broadcast → refresh confirms |
| `live_send_insufficient_funds` | Requesting > balance → `insufficient funds` before draft persist |
| `live_send_dust_amount_rejected` | Amount below `DUST_THRESHOLD` → rejected pre-selection |
| `live_send_change_lands_on_change_address` | External recipient → change appears at branch=1/idx=0 |
| `live_send_dust_change_folded_into_fee` | Sub-dust remainder folds into fee (`change==0`), tx accepted |
| `live_send_max_sweeps_all_coins` | `max=true` sweeps every coin, no change, single output |
| `live_send_immature_coinbase_rejected_then_matures` | Guards commit `a520456`: immature coinbase not spendable until `height+maturity ≤ tip+1` |
| `live_send_wrong_network_address_rejected` | Mainnet `hs1q…` on regtest → rejected before signing |
| `live_send_double_spend_is_not_sent` | A draft whose coin another mined draft spent is not sent: hsd answers its txid, the look-up after it finds nothing, the broadcast errors "did not take" and the draft is `broadcast_pending`; it is never mined |
| `live_send_rebroadcast_same_draft_rejected` | A mined draft is not sent again: refused "already sent" before anything reaches the node, still `confirmed` |
| `live_send_estimate_fee_smoke` | `estimate_tx_draft_fee` matches a real build's fee/change/inputs |
| `live_send_txid_matches_node` | Local txid == node-returned txid (sighash/serialization guard) |

### Reservation invariants (Group B)

| Test | Asserts |
|------|---------|
| `live_reservation_excludes_other_draft` | Second build can't select the first draft's reserved coin |
| `live_reservation_ttl_reclaim` | Stale unsigned draft's reservation is reclaimed; broadcasted is NOT |
| `live_delete_draft_releases_and_guards` | `delete_tx_draft` frees unsigned; refuses broadcasted/confirmed |
| `live_release_reservation_command` | `release_tx_draft_reservation` frees coins without deleting draft |

### Confirmation / reorg state machine (Group C)

These drive REAL reorgs via a `#[cfg(test)]` `invalidateblock`/`reconsiderblock` RPC passthrough (`NodeRpcClient`).

| Test | Asserts |
|------|---------|
| `live_confirm_reorg_reverts_to_broadcasted` | Confirmed → invalidateblock → refresh reverts → reconsider → re-confirms |
| `live_confirm_finality_ceiling` | ≥ `CONFIRMATION_FINALITY_DEPTH` confs → refresh stops churning |
| `live_broadcast_pending_promotes` | `broadcast_pending` draft on-chain → refresh promotes via `local_txid_from_summary` |
| `live_coinbase_reorg_immaturity` | Unmine a confirmed coinbase → wallet treats the re-mined one as freshly immature |
| `live_coins_spent_by_another_tx_drop_the_draft_and_say_so` | A draft whose coin another mined transaction spent is not sent (`broadcast_pending`), then `dropped` after the grace window, saying another transaction spent its coins, never "the coins were not moved" |
| `live_dropped_send_mined_after_all_is_confirmed` | A send judged `dropped` (coins released) that is then mined is `confirmed` at its block by the next poll |

### A node without a transaction index (Group C, second node)

hsd finds a transaction by txid only in its mempool and its transaction index, so on a node without `--index-tx` every mined transaction reads as hsd's "Transaction not found.". These tests need a second regtest node started with the address index alone, on ports of its own; they are skipped while `HNS_IT_NOINDEX_NODE_URL` is unset or empty:

```bash
hsd --network=regtest --index-address --no-wallet --listen=false \
    --http-host=127.0.0.1 --http-port=24037 --port=24038 \
    --ns-port=25449 --rs-port=25450 --api-key=test --prefix=<a fresh dir> --daemon
HNS_IT_NOINDEX_NODE_URL=http://127.0.0.1:24037 HNS_IT_NOINDEX_NODE_API_KEY=test \
  cargo test --manifest-path src-tauri/Cargo.toml live_noindex -- --test-threads=1
```

| Test | Asserts |
|------|---------|
| `live_noindex_mined_send_is_confirmed_not_dropped` | A mined send is `confirmed` at its block after the grace window, and stays so on the next poll; never `dropped` |
| `live_noindex_mined_pending_broadcast_is_confirmed_not_failed` | A mined `broadcast_pending` draft is `confirmed` with its txid; never `failed` |
| `live_noindex_unsent_draft_with_unspent_coins_is_dropped` | A draft the node never had, its coins unspent, is `dropped` and its coins released |
| `live_noindex_coins_spent_by_another_tx_give_no_verdict` | A draft whose coin another transaction spent is not sent and stays `broadcast_pending`: without the index the wallet cannot tell that from a mined one |
| `live_noindex_dropped_send_mined_after_all_is_confirmed` | A send judged `dropped` that is then mined is `confirmed`, found by its outputs |

### Covenant actions (Group D)

| Test | Asserts |
|------|---------|
| `live_update_records` | UPDATE writes records; name stays CLOSED, owner value preserved |
| `live_renew_extends_lease` | RENEW straight after REGISTER is refused (`bad-renewal-premature`: not before `renewal + tree_interval`); after that many blocks it lands, `renewal` moves to the block it was mined in and the value is kept. Guards commit `78bba67`: RENEW uses `getblockhash` UNREVERSED (no `bad-register-renewal`) |
| `live_cancel_reverts_transfer` | CANCEL clears a pending transfer; name still owned |
| `live_revoke_burns_control` | REVOKE → name state REVOKED |

### Batch covenant variants (Group E)

| Test | Asserts |
|------|---------|
| `live_batch_bid_two_names` | Shared-lockup batch bid → both BIDs land, commitments persisted |
| `live_batch_reveal_two_names` | Batch reveal after batch bid → both reveals accepted |
| `live_batch_redeem_two_names` | A rival wallet outbids this one on both names; one batch REDEEM spends both losing reveals, one REDEEM output per name, and the rival's reveals stay the owners |
| `live_batch_renew_two_names` | Batch refused while the last-registered name is too recently renewed; after the tree interval, shared `renewal_block` → both names' `renewal` moves to the block it was mined in |
| `live_batch_large_covenant_count` | 20 names renewed in one transaction once the last one may be renewed; every name's `renewal` moves to that block |
| `live_batch_finalize_two_names` | Single-tx batch finalize after batch transfer + lockup |

### Covenant invariant + timing negatives (Group F)

| Test | Asserts |
|------|---------|
| `live_premature_finalize_rejected` | Finalize before `transfer_lockup` (10) → the builder refuses it ("still locked for 9 more blocks") and persists nothing, as hsd would refuse it (`bad-finalize-maturity`); after → accepted and the name is at the recipient |
| `live_bid_lockup_invariant_and_reveal_value` | On-chain: BID value == lockup; REVEAL value == true bid |
| `live_register_value_is_clearing_price` | REGISTER output value == `getnameinfo.info.value` |
| `live_redeem_when_won_rejected` | Winner cannot `build_redeem_draft` (no losing reveal to reclaim) |
| `live_redeem_of_the_winning_reveal_is_refused` | A lone bidder's reveal, before REGISTER and with the name never tracked, is the owner coin the node reports: `build_redeem_draft` refuses it ("won the auction"), as hsd would (`bad-redeem-owner`), and the coin stays unspent |
| `live_double_open_and_double_bid_guarded` | Second OPEN/BID while first pending → command-level rejection |

### Atomic swap + signing + capability (Group G)

| Test | Asserts |
|------|---------|
| `live_finalize_with_payment_atomic` | One tx carries BOTH a FINALIZE covenant AND the seller-payment output |
| `live_sign_name_message_ownership_gate` | `sign_name_message` signs an owned name; refuses one the wallet does not own |
| `live_signer_profile_mismatch_refused` | Signer unlocked for profile X → refuses to sign profile Y's draft |
| `live_watch_only_send_refused` | Watch-only profile → `build_send_hns_draft` refuses |
| `live_write_capability_downgrades` | Node reachable + local + signer unlocked → `can_write`; flip to Explorer → refused |

## Shakedex purchases against the CLI

These are manual checks that Namehold buys from the real shakedex CLI and meets it as the other party (spec R30, Namehold-buys half, and the R7/R10/R13 chain paths). They are not part of CI: each test is skipped while `HNS_IT_SHAKEDEX` is unset or empty.

Start a throwaway regtest node with the hsd wallet enabled; the default `scripts/regtest.sh start` runs with `--no-wallet`, and the script prints the wallet API port and key (14039, `test`). The node uses the repo-local `.regtest/` data dir and never touches `~/.hsd`. The CLI talks to the node at the regtest default ports, so point the tests at that node.

```bash
scripts/regtest.sh --with-wallet
HNS_IT_NODE_URL=http://127.0.0.1:14037 HNS_IT_NODE_API_KEY=test HNS_IT_SHAKEDEX=1 \
  cargo test --manifest-path src-tauri/Cargo.toml --lib live_node_it::shakedex_ -- --test-threads=1 --nocapture
```

The tests drive the CLI through `scripts/shakedex-cli-sell.sh`, with the node's hsd wallet as the seller and as the other buyer. Each run registers fresh names (open, two bids, reveal, register, mining between the phases), so a used chain works, up to a point: regtest halves the block subsidy every 2500 blocks (`halvingInterval`), and the tests fund their wallets by mining, so a chain some tens of thousands of blocks tall leaves them too little to spend. A full run of the live suite mines about 10 000 blocks (the Shakedex tests about 3000 of them, registering names and funding buyers), so start over after two runs; the Shakedex buyer refuses to start on a chain past 30 000 blocks and says why; `scripts/regtest.sh reset` starts over. Each test funds an account of its own (`seeded_conn_regtest` hands out a fresh one): the tests once shared account 0, which gathered thousands of coinbases a run, until a sweep of it outgrew a standard transaction and stalled every later test behind an unmined transaction. The R9 test below mines about 5000 blocks and takes them back out with `invalidateblock` before it asserts anything, so it leaves the chain as tall as it found it. The R31 test (`shakedex_lock_refused_near_expiry`) does the same toward its own name's expiry, mining to an address no profile derives, and a guard takes the blocks back out even if the test panics first. The T2 lock tests (`shakedex_lock_*`, `shakedex_cancel_transfer_aborts_the_listing_on_chain`) are gated by `HNS_IT_SHAKEDEX` like the rest, but use no CLI: they register a name with the wallet itself, so they also run against a node at another port. The script clones `shadstoneofficial/shakedex` at the pinned commit `2c4fa04eab68a528e758598d11b5da5666113b11` into `SHAKEDEX_WORK` once and keeps the CLI's database there (the tests default it to `$TMPDIR/namehold-shakedex-cli`; run by hand, the script uses a fresh temp dir unless it is set); nothing is installed globally. `HNS_IT_SHAKEDEX` set to anything but `1` or empty, or set to `1` without `HNS_IT_NODE_URL`, or with a node URL other than the regtest RPC port (14037), fails the tests instead of skipping them. It can be run by hand too:

| Command | What the CLI does |
|---|---|
| `REGISTER=1 PRICE=5 OUT=listing.json scripts/shakedex-cli-sell.sh` | registers a name, locks it (`transfer-lock`, the lockup, `finalize-lock`) and lists it at a fixed price (`create-fixed`); prints the listing path |
| `REGISTER=1 START_PRICE=10 END_PRICE=5 OUT=auction.json scripts/shakedex-cli-sell.sh auction` | the same, listed as a one-day reverse auction, one step every 15 minutes (`create-auction`, not published) |
| `SHAKEDEX_WORK=<dir> scripts/shakedex-cli-sell.sh cancel <name>` | takes the name back out of its lock (`transfer-lock-cancel`, the lockup, `finalize-lock-cancel`) and checks on the node that it is back in the hsd wallet; refuses to run without the `SHAKEDEX_WORK` the listing was made in |
| `scripts/shakedex-cli-sell.sh fill <listing.json>` | the hsd wallet buys the listing (`fill-auction`) and checks on the node that the name is being transferred |
| `scripts/shakedex-cli-sell.sh register` | registers a fresh name with the hsd wallet and prints it (no CLI needed) |
| `scripts/shakedex-cli-sell.sh lock-to <name> <address>` | the hsd wallet transfers the name to the address and finalizes it there, as the CLI's `transfer-lock`/`finalize-lock` do; prints the owner outpoint (no CLI needed) |

What the tests check:

- `shakedex_cli_listing_is_bought` — a fixed-price listing is bought: unconfirmed in the mempool, awaiting finalize once mined, owned after the lockup and the FINALIZE, with the node's owner coin paying the purchase's destination.
- `shakedex_finalized_name_moved_on_before_a_sync_is_owned` — the purchase is finalized and the name then sent elsewhere (TRANSFER, lockup, FINALIZE) before the purchase job runs again; the job finds our FINALIZE as the TRANSFER's spender in the node's history of our destination and marks the purchase owned. The shakedex lock script lets the TRANSFER be spent only into a FINALIZE, so there is no other ending to test.
- `shakedex_cli_buyer_first_loses_ours_with_nothing_paid` — the CLI buys the listing between our review and our broadcast. hsd 8.0.0's `sendrawtransaction` answers with the txid anyway; the look-up after it finds the node did not take the purchase, so the broadcast says "did not take" and the draft waits as `broadcast_pending`. The purchase job then finds the lock coin spent and loses the purchase with nothing paid, its coins free again, and the listing is no longer offered.
- `shakedex_purchase_follows_reorgs_of_its_own_blocks` — the purchase's block is invalidated (hsd's `invalidateblock` empties the mempool too, so the purchase is gone from the node), the app's sync rebroadcasts it once after six blocks missing, and it is mined again; then the FINALIZE's block is invalidated and the purchase awaits finalize again until the same FINALIZE is mined.
- `shakedex_reverse_auction_pays_the_current_step_and_refuses_a_stale_one` — a purchase signed at one step is refused at broadcast once the node's median time makes a cheaper step valid, and built again it pays that step.
- `shakedex_rebroadcast_after_a_price_drop_is_not_sent` — the purchase's block is invalidated, which leaves it nowhere; a cheaper step of the reverse auction becomes valid before its one rebroadcast is due, so the old price is not sent again: the purchase is lost with nothing paid, its coins free, and the node never sees it again.
- `shakedex_cancelled_listing_is_not_offered` — after the seller cancels, the listing is "sold or cancelled" and cannot be bought.
- `shakedex_listing_expiring_before_finalize_is_not_offered` — R9 at the boundary: the chain is mined until the name's expiry (hsd's own `renewalPeriodEnd`) is one block past the margin (tip + 1 + transfer lockup + one day, a day being the lockup on regtest), where the listing is buyable with the expiry warning; one block later it is "expires before it can be finalized" and building the purchase is refused. It mines about 5000 blocks, regtest's renewal window.

Some listings the CLI cannot write: it publishes a market fee only through LearnHNS, starts every listing at the node's median time (so its first step is valid at once), and never writes two steps at one price unless asked to. For those the tests hold the lock key themselves (`OwnLock`): the hsd wallet registers a name and moves it to a Shakedex lock address of a test key with `lock-to`, and the test signs the price steps the way the CLI does.

- `shakedex_accepted_market_fee_is_paid_exactly_on_chain` — a listing with a market fee the buyer accepts: the mined purchase pays the seller the price and the fee address exactly the fee, as hsd reports the outputs.
- `shakedex_listing_not_valid_yet_becomes_buyable` — a step an hour ahead of the median time is "not valid yet" and cannot be bought; once the median time passes it, it is buyable.
- `shakedex_purchase_change_is_held_back_until_seen_mined` — the purchase's change, tracked once mined, stays out of coin selection until the purchase job has seen the purchase mined.
- `shakedex_name_expired_before_finalize_is_lost_with_the_price_paid` — R13 "Lost, paid": the chain is mined to the name's expiry (hsd then reports `info: null`), and the purchase is lost as paid; the blocks are taken back before anything is asserted.
- `shakedex_purchase_at_a_same_price_step_not_valid_is_not_sent` — two steps at one price, the purchase signed at the later; the blocks that moved the median time past it are taken back, so only the earlier step is valid, and the purchase is refused before it is sent. Checking the price alone sent it, and hsd answered with its txid although it could only hold it as non-final.
- `shakedex_purchase_the_node_lost_is_given_up_after_mempool_expiry` — the purchase's block is invalidated (the node then has nothing of it); with the daemon's no-resend rule it waits, still unconfirmed one block before hsd's mempool expiry and lost with nothing paid at it, and nothing is sent again.

`live_send_pays_its_fee_rate_on_vsize` (no CLI needed) checks a send against the rate hsd itself reports for it, worked out on the virtual size. `shakedex_purchase_and_finalize_pay_their_fee_rate_on_vsize` does the same for a purchase and its FINALIZE built at 20 doos/byte, above the 5 doos/byte floor every other test's rate falls under.

The tests move the node's clock forward with `setmocktime` to make price steps valid. hsd keeps that as an offset that goes on ticking, and the median time never goes back, so the clock is only ever moved forward; never run `setmocktime 0` against this node, which sets its clock to 0 and stalls mining (`scripts/regtest.sh reset` starts over).

The winner of a name pays the second-highest bid, so the preamble places two bids to give the lock coin a non-zero value (0.5 HNS). A name won uncontested has a lock coin worth 0, which Namehold accepts as well. Stop the node with `scripts/regtest.sh stop`, or wipe the chain with `scripts/regtest.sh reset`.
