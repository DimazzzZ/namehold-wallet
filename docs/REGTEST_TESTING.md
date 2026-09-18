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
| 3 | Finalize the transfer | FINALIZE tx broadcasted; name moves to recipient |
| 4 | Verify original wallet no longer `ownsName` | `capabilities.ownsName` is `false` |

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
| `live_auction_open_bid_reveal_redeem` | Loser redeem: OPEN → BID → REVEAL → REDEEM |
| `live_auction_register_transfer_finalize` | Post-win: REGISTER → TRANSFER → FINALIZE |
| `live_batch_transfer_two_names` | Batch: acquire 2 → BATCH TRANSFER → FINALIZE each |

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
| `live_send_broadcast_double_spend_releases_reservation` | Double-spend broadcast → `failed` status, reservation freed |
| `live_send_rebroadcast_same_draft_rejected` | Second broadcast of a mined draft → node rejects (no RBF) |
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

### Covenant actions (Group D)

| Test | Asserts |
|------|---------|
| `live_update_records` | UPDATE writes records; name stays CLOSED, owner value preserved |
| `live_renew_extends_lease` | Guards commit `78bba67`: RENEW uses `getblockhash` UNREVERSED (no `bad-register-renewal`) |
| `live_cancel_reverts_transfer` | CANCEL clears a pending transfer; name still owned |
| `live_revoke_burns_control` | REVOKE → name state REVOKED |

### Batch covenant variants (Group E)

| Test | Asserts |
|------|---------|
| `live_batch_bid_two_names` | Shared-lockup batch bid → both BIDs land, commitments persisted |
| `live_batch_reveal_two_names` | Batch reveal after batch bid → both reveals accepted |
| `live_batch_redeem_two_names` | Losing batch → batch redeem reclaims both lockups |
| `live_batch_renew_two_names` | Shared `renewal_block` → both renewed |
| `live_batch_finalize_two_names` | Single-tx batch finalize after batch transfer + lockup |

### Covenant invariant + timing negatives (Group F)

| Test | Asserts |
|------|---------|
| `live_premature_finalize_rejected` | Finalize before `transfer_lockup` (10) → rejected; after → accepted |
| `live_bid_lockup_invariant_and_reveal_value` | On-chain: BID value == lockup; REVEAL value == true bid |
| `live_register_value_is_clearing_price` | REGISTER output value == `getnameinfo.info.value` |
| `live_redeem_when_won_rejected` | Winner cannot `build_redeem_draft` (no losing reveal to reclaim) |
| `live_double_open_and_double_bid_guarded` | Second OPEN/BID while first pending → command-level rejection |

### Atomic swap + signing + capability (Group G)

| Test | Asserts |
|------|---------|
| `live_finalize_with_payment_atomic` | One tx carries BOTH a FINALIZE covenant AND the seller-payment output |
| `live_sign_name_message_ownership_gate` | `sign_name_message` signs an owned name; refuses one the wallet does not own |
| `live_signer_profile_mismatch_refused` | Signer unlocked for profile X → refuses to sign profile Y's draft |
| `live_watch_only_send_refused` | Watch-only profile → `build_send_hns_draft` refuses |
| `live_write_capability_downgrades` | Node reachable + local + signer unlocked → `can_write`; flip to Explorer → refused |
